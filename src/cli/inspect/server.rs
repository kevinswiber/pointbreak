//! Minimal synchronous HTTP/1.1 server for the `.shore` inspector.
//!
//! This is deliberately small and blocking: one OS thread per connection,
//! `Connection: close` responses, GET-only routing. It introduces no async
//! runtime and no third-party HTTP crate, in keeping with the storage-model
//! rule against pulling in a runtime before a remote backend forces it. It is
//! a localhost developer tool, not a production server.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::api;

const INDEX_HTML: &str = include_str!("assets/index.html");
const APP_CSS: &str = include_str!("assets/app.css");
const APP_JS: &str = include_str!("assets/app.js");

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn new(status: &'static str, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type,
            body,
        }
    }

    fn asset(content_type: &'static str, body: &str) -> Self {
        Self::new("200 OK", content_type, body.as_bytes().to_vec())
    }

    fn json_ok(body: String) -> Self {
        Self::new(
            "200 OK",
            "application/json; charset=utf-8",
            body.into_bytes(),
        )
    }

    fn json_error(status: &'static str, message: &str) -> Self {
        let body = serde_json::json!({ "error": message }).to_string();
        Self::new(status, "application/json; charset=utf-8", body.into_bytes())
    }

    fn text(status: &'static str, message: &str) -> Self {
        Self::new(
            status,
            "text/plain; charset=utf-8",
            message.as_bytes().to_vec(),
        )
    }
}

pub(super) fn serve(
    addr: SocketAddr,
    repo: PathBuf,
    open: bool,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener =
        TcpListener::bind(addr).map_err(|error| format!("could not bind {addr}: {error}"))?;
    // Resolve the actually-bound address so an ephemeral port (`--port 0`)
    // is shown and opened correctly rather than `:0`.
    let bound = listener.local_addr().unwrap_or(addr);
    let url = format!("http://{bound}/");

    writeln!(stdout, "shore inspector")?;
    writeln!(stdout, "  store: {}", repo.display())?;
    writeln!(stdout, "  url:   {url}")?;
    writeln!(stdout, "  stop:  Ctrl-C")?;
    stdout.flush().ok();

    if open {
        open_browser(&url);
    }

    let repo = Arc::new(repo);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let repo = Arc::clone(&repo);
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &repo) {
                        tracing::debug!(error = %error, "inspect_connection_error");
                    }
                });
            }
            Err(error) => {
                tracing::debug!(error = %error, "inspect_accept_error");
            }
        }
    }

    Ok(())
}

fn handle_connection(stream: TcpStream, repo: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;

    let read_half = stream.try_clone()?;
    let mut reader = BufReader::new(read_half);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }

    // Drain request headers; we do not consume request bodies (GET-only API).
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");
    let (path, query) = split_target(target);

    let response = route(repo, method, path, query);
    write_response(stream, &response)
}

fn route(repo: &Path, method: &str, path: &str, query: Option<&str>) -> Response {
    if method != "GET" {
        return Response::text("405 Method Not Allowed", "method not allowed");
    }

    match path {
        "/" | "/index.html" => Response::asset("text/html; charset=utf-8", INDEX_HTML),
        "/app.css" => Response::asset("text/css; charset=utf-8", APP_CSS),
        "/app.js" => Response::asset("application/javascript; charset=utf-8", APP_JS),
        "/api/history" => api_response(api::history_json(repo)),
        "/api/units" => api_response(api::units_json(repo)),
        "/api/freshness" => api_response(api::freshness_json(repo)),
        "/api/snapshot" => match query_param(query, "id") {
            Some(id) if !id.is_empty() => api_response(api::snapshot_json(repo, &id)),
            _ => Response::json_error("400 Bad Request", "missing ?id=<snapshotId>"),
        },
        "/api/unit" => match query_param(query, "id") {
            Some(id) if !id.is_empty() => api_response(api::unit_json(repo, &id)),
            _ => Response::json_error("400 Bad Request", "missing ?id=<reviewUnitId>"),
        },
        "/favicon.ico" => Response::new("204 No Content", "image/x-icon", Vec::new()),
        _ => Response::json_error("404 Not Found", "no such route"),
    }
}

fn split_target(target: &str) -> (&str, Option<&str>) {
    match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (target, None),
    }
}

fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some(key) {
            return Some(percent_decode(kv.next().unwrap_or("")));
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi * 16 + lo) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn api_response(result: Result<String, String>) -> Response {
    match result {
        Ok(body) => Response::json_ok(body),
        Err(message) => Response::json_error("500 Internal Server Error", &message),
    }
}

fn write_response(mut stream: TcpStream, response: &Response) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len(),
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let command = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let command = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let command = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();

    if let Err(error) = command {
        tracing::debug!(error = %error, "inspect_open_browser_failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route_for(method: &str, path: &str) -> Response {
        // Static assets, 404, 405, and the missing-id snapshot case do not
        // touch the store, so the repo path is never read for these cases.
        route(
            Path::new("/inspect-routing-test-unused"),
            method,
            path,
            None,
        )
    }

    #[test]
    fn root_serves_index_html() {
        let response = route_for("GET", "/");
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        assert!(!response.body.is_empty());
    }

    #[test]
    fn static_assets_carry_expected_content_types() {
        assert_eq!(
            route_for("GET", "/app.css").content_type,
            "text/css; charset=utf-8"
        );
        assert_eq!(
            route_for("GET", "/app.js").content_type,
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn unknown_route_is_json_not_found() {
        let response = route_for("GET", "/does-not-exist");
        assert_eq!(response.status, "404 Not Found");
        assert!(response.content_type.starts_with("application/json"));
    }

    #[test]
    fn non_get_methods_are_rejected() {
        assert_eq!(
            route_for("POST", "/api/history").status,
            "405 Method Not Allowed"
        );
    }

    #[test]
    fn snapshot_without_id_is_bad_request() {
        assert_eq!(route_for("GET", "/api/snapshot").status, "400 Bad Request");
    }

    #[test]
    fn unit_without_id_is_bad_request() {
        assert_eq!(route_for("GET", "/api/unit").status, "400 Bad Request");
    }

    #[test]
    fn query_param_reads_and_percent_decodes_values() {
        let query = Some("id=snap%3Agit%3Asha256%3Aabc&other=1");
        assert_eq!(
            query_param(query, "id").as_deref(),
            Some("snap:git:sha256:abc")
        );
        assert_eq!(query_param(query, "missing"), None);
        assert_eq!(query_param(None, "id"), None);
    }

    #[test]
    fn split_target_separates_path_and_query() {
        assert_eq!(
            split_target("/api/snapshot?id=x"),
            ("/api/snapshot", Some("id=x"))
        );
        assert_eq!(split_target("/"), ("/", None));
    }
}

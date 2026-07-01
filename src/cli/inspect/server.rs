//! Minimal synchronous HTTP/1.1 server for the `.shore` inspector.
//!
//! This is deliberately small and blocking: one OS thread per connection,
//! `Connection: close` responses, GET-only routing. It introduces no async
//! runtime and no third-party HTTP crate, in keeping with the storage-model
//! rule against pulling in a runtime before a remote backend forces it. It is
//! a localhost developer tool, not a production server.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use shoreline::model::EventId;
use shoreline::session::{HistoryOrder, HistoryPage, HistoryQuery};

use super::api;

/// Shared, read-only inspector server state. Holds the resolved store path and the read-time
/// highlight cache; cloned cheaply behind an `Arc` to every connection thread.
pub(super) struct InspectState {
    pub repo: PathBuf,
    pub highlight_cache: RwLock<HighlightCache>,
    /// The single-slot base-projection cache (#255): repeated `/api/history`
    /// queries over one store version reuse the fully-hydrated base instead of
    /// re-reading and re-hydrating the whole log per request (INV-5).
    pub history_cache: super::cache::HistoryProjectionCache,
}

impl InspectState {
    pub(super) fn new(repo: PathBuf) -> Self {
        Self {
            repo,
            highlight_cache: RwLock::new(HighlightCache::new(HIGHLIGHT_CACHE_CAPACITY)),
            history_cache: super::cache::HistoryProjectionCache::new(),
        }
    }
}

/// How many `snapshot_json` responses to retain. Snapshots are opened on demand (not polled), so a
/// small cap amortizes repeat opens without holding the whole history.
const HIGHLIGHT_CACHE_CAPACITY: usize = 64;

/// A bounded, content-hash-keyed cache of fully-rendered `snapshot_json` responses. Eviction is
/// always safe: the value is recomputable from the content-addressed artifact, so there is no
/// invalidation — entries only age out by insertion order once the cap is reached.
pub(super) struct HighlightCache {
    cap: usize,
    map: HashMap<String, String>,
    order: Vec<String>,
}

impl HighlightCache {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            cap,
            map: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub(super) fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }

    pub(super) fn put(&mut self, key: &str, value: String) {
        if self.map.contains_key(key) {
            self.map.insert(key.to_owned(), value);
            return;
        }
        if self.cap == 0 {
            return;
        }
        while self.order.len() >= self.cap {
            let evicted = self.order.remove(0);
            self.map.remove(&evicted);
        }
        self.order.push(key.to_owned());
        self.map.insert(key.to_owned(), value);
    }
}

const INDEX_HTML: &str = include_str!("assets/index.html");
const TOKENS_CSS: &str = include_str!("assets/tokens.css");
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

    let state = Arc::new(InspectState::new(repo));
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &state) {
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

fn handle_connection(stream: TcpStream, state: &InspectState) -> std::io::Result<()> {
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

    let response = route(state, method, path, query);
    write_response(stream, &response)
}

fn route(state: &InspectState, method: &str, path: &str, query: Option<&str>) -> Response {
    if method != "GET" {
        return Response::text("405 Method Not Allowed", "method not allowed");
    }

    let repo = state.repo.as_path();
    match path {
        "/" | "/index.html" => Response::asset("text/html; charset=utf-8", INDEX_HTML),
        "/tokens.css" => Response::asset("text/css; charset=utf-8", TOKENS_CSS),
        "/app.css" => Response::asset("text/css; charset=utf-8", APP_CSS),
        "/app.js" => Response::asset("application/javascript; charset=utf-8", APP_JS),
        "/api/history" => match history_query(query) {
            Ok(request) => api_response(api::history_json(
                repo,
                &state.history_cache,
                &request.query,
                &request.page,
            )),
            Err(message) => Response::json_error("400 Bad Request", &message),
        },
        "/api/revisions" => api_response(api::revisions_json(repo)),
        "/api/threads" => api_response(api::threads_json(repo)),
        "/api/freshness" => api_response(api::freshness_json(repo)),
        "/favicon.ico" => Response::new("204 No Content", "image/x-icon", Vec::new()),
        _ => route_member(state, path, query),
    }
}

/// Path-member routes: `/api/revisions/{id}` and `/api/snapshots/{id}`. An empty
/// member (a trailing slash with no id) is a `400`; anything else unmatched is a
/// `404`. The id segment arrives percent-encoded (the client encodes it with
/// `encodeURIComponent`) and is decoded here.
fn route_member(state: &InspectState, path: &str, query: Option<&str>) -> Response {
    let repo = state.repo.as_path();
    if let Some(raw) = path_member(path, "/api/revisions/") {
        return match decode_member(raw) {
            Some(id) => api_response(api::revision_json(repo, &id)),
            None => Response::json_error("400 Bad Request", "missing revision id"),
        };
    }
    if let Some(raw) = path_member(path, "/api/snapshots/") {
        return match decode_member(raw) {
            Some(id) => {
                let content_hash = query_param(query, "contentHash");
                api_response(api::snapshot_json(
                    repo,
                    &id,
                    content_hash.as_deref(),
                    Some(&state.highlight_cache),
                ))
            }
            None => Response::json_error("400 Bad Request", "missing snapshot id"),
        };
    }
    Response::json_error("404 Not Found", "no such route")
}

/// The single path segment after `prefix` (e.g. `/api/revisions/`), still
/// percent-encoded. `None` when `path` is not under `prefix` or the remainder
/// spans more than one segment (a literal `/`).
fn path_member<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(prefix)?;
    if rest.contains('/') {
        return None;
    }
    Some(rest)
}

/// Percent-decode a captured path member into the id, or `None` when it is empty.
fn decode_member(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    Some(percent_decode(raw))
}

fn split_target(target: &str) -> (&str, Option<&str>) {
    match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (target, None),
    }
}

/// The parsed `/api/history` request: the query model and the window spec.
struct HistoryRequest {
    query: HistoryQuery,
    page: HistoryPage,
}

/// Parse the `/api/history` query params into a `HistoryQuery` + `HistoryPage`.
/// `q` is free text; `track`/`object`/`at` are exact (empty => absent); `type` is
/// a comma-separated enabled-type set (absent => all types); `order` is
/// `asc`/`desc` (absent/empty => asc). A non-numeric `limit`/`offset` or an unknown
/// `order` is a usage error the caller turns into a `400` without touching the
/// store. The `at` › `offset` precedence lives in `apply_history_query`; the parser
/// only collects the params. Paging is positional (`offset`/`at`); the opaque
/// forward cursor stays on the CLI path (`shore review history --cursor`).
fn history_query(query: Option<&str>) -> Result<HistoryRequest, String> {
    let q = query_param(query, "q").unwrap_or_default();
    let track = query_param(query, "track").filter(|value| !value.is_empty());
    let object = query_param(query, "object").filter(|value| !value.is_empty());
    let types = query_param(query, "type").map(|raw| {
        raw.split(',')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<String>>()
    });
    let order = match query_param(query, "order").as_deref() {
        None | Some("") | Some("asc") => HistoryOrder::Asc,
        Some("desc") => HistoryOrder::Desc,
        Some(_) => return Err("invalid order".to_owned()),
    };
    let limit = parse_usize(query_param(query, "limit"), "invalid limit")?;
    let offset = parse_usize(query_param(query, "offset"), "invalid offset")?;
    let at = query_param(query, "at")
        .filter(|value| !value.is_empty())
        .map(EventId::new);
    Ok(HistoryRequest {
        query: HistoryQuery {
            q,
            track,
            object,
            types,
            order,
        },
        page: HistoryPage { limit, offset, at },
    })
}

/// Parse an optional numeric query param; a present but non-numeric value is a
/// usage error (`message`), an absent one is `None`.
fn parse_usize(value: Option<String>, message: &'static str) -> Result<Option<usize>, String> {
    match value {
        Some(raw) => Ok(Some(raw.parse::<usize>().map_err(|_| message.to_owned())?)),
        None => Ok(None),
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
        let state = InspectState::new(PathBuf::from("/inspect-routing-test-unused"));
        route(&state, method, path, None)
    }

    #[test]
    fn snapshot_cache_returns_identical_bytes_on_hit() {
        let mut cache = HighlightCache::new(8);
        assert!(cache.get("sha256:abc").is_none()); // miss
        let body = "{\"snapshot\":1}".to_owned();
        cache.put("sha256:abc", body.clone());
        assert_eq!(cache.get("sha256:abc").as_deref(), Some(body.as_str())); // hit
    }

    #[test]
    fn highlight_cache_evicts_oldest_past_capacity() {
        let mut cache = HighlightCache::new(2);
        cache.put("a", "1".to_owned());
        cache.put("b", "2".to_owned());
        cache.put("c", "3".to_owned()); // evicts the oldest entry, "a"
        assert!(cache.get("a").is_none());
        assert_eq!(cache.get("b").as_deref(), Some("2"));
        assert_eq!(cache.get("c").as_deref(), Some("3"));
    }

    #[test]
    fn root_serves_index_html() {
        let response = route_for("GET", "/");
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        assert!(!response.body.is_empty());
    }

    #[test]
    fn app_css_styles_verification_and_endorsement_readback() {
        let response = route_for("GET", "/app.css");
        let body = String::from_utf8(response.body).expect("app.css is utf-8");
        assert!(
            body.contains(".verify") && body.contains(".endorsements"),
            "app.css carries the verification chip and endorsement block styles"
        );
    }

    #[test]
    fn static_assets_carry_expected_content_types() {
        assert_eq!(
            route_for("GET", "/tokens.css").content_type,
            "text/css; charset=utf-8"
        );
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
    fn path_member_extracts_single_segment() {
        assert_eq!(
            path_member("/api/revisions/abc", "/api/revisions/"),
            Some("abc")
        );
        assert_eq!(
            path_member("/api/snapshots/x%3Ay", "/api/snapshots/"),
            Some("x%3Ay")
        );
        // A deeper path is not a single member.
        assert_eq!(path_member("/api/revisions/a/b", "/api/revisions/"), None);
        // No trailing slash: the collection, not a member.
        assert_eq!(path_member("/api/revisions", "/api/revisions/"), None);
        // Trailing slash, empty member.
        assert_eq!(path_member("/api/revisions/", "/api/revisions/"), Some(""));
    }

    #[test]
    fn decode_member_percent_decodes_nonempty() {
        assert_eq!(
            decode_member("snap%3Agit%3Asha256%3Aabc").as_deref(),
            Some("snap:git:sha256:abc")
        );
        assert_eq!(decode_member(""), None);
    }

    #[test]
    fn revisions_member_without_id_is_bad_request() {
        assert_eq!(
            route_for("GET", "/api/revisions/").status,
            "400 Bad Request"
        );
    }

    #[test]
    fn snapshots_member_without_id_is_bad_request() {
        assert_eq!(
            route_for("GET", "/api/snapshots/").status,
            "400 Bad Request"
        );
    }

    #[test]
    fn bare_snapshots_collection_is_not_found() {
        // There is no snapshot-list endpoint; only `/api/snapshots/{id}` exists.
        assert_eq!(route_for("GET", "/api/snapshots").status, "404 Not Found");
    }

    #[test]
    fn deeper_member_paths_are_not_found() {
        assert_eq!(
            route_for("GET", "/api/revisions/a/b").status,
            "404 Not Found"
        );
        assert_eq!(
            route_for("GET", "/api/threads/anything").status,
            "404 Not Found"
        );
    }

    #[test]
    fn retired_routes_are_not_found() {
        // The pre-reshape object/singular routes and the older lineage routes.
        for path in [
            "/api/objects",
            "/api/object",
            "/api/revision",
            "/api/lineages",
            "/api/lineage",
        ] {
            assert_eq!(
                route_for("GET", path).status,
                "404 Not Found",
                "{path} is retired"
            );
        }
    }

    #[test]
    fn query_param_reads_and_percent_decodes_values() {
        let query = Some("contentHash=snap%3Agit%3Asha256%3Aabc&other=1");
        assert_eq!(
            query_param(query, "contentHash").as_deref(),
            Some("snap:git:sha256:abc")
        );
        assert_eq!(query_param(query, "missing"), None);
        assert_eq!(query_param(None, "contentHash"), None);
    }

    #[test]
    fn split_target_separates_path_and_query() {
        assert_eq!(
            split_target("/api/snapshots/x?contentHash=y"),
            ("/api/snapshots/x", Some("contentHash=y"))
        );
        assert_eq!(split_target("/"), ("/", None));
    }
}

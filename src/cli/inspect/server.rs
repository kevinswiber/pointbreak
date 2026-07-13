//! Minimal synchronous HTTP/1.1 server for the `.shore` inspector.
//!
//! This is deliberately small and blocking: one OS thread per connection,
//! `Connection: close` responses, GET-only routing. It introduces no async
//! runtime and no third-party HTTP crate, in keeping with the storage-model
//! rule against pulling in a runtime before a remote backend forces it. It is
//! a localhost developer tool, not a production server.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{fmt, thread};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pointbreak::documents::{InspectStartupDocument, version_document};
use pointbreak::model::EventId;
use pointbreak::session::{
    HistoryOrder, HistoryPage, HistoryQuery, QueryDiagnosticCode, QuerySurface,
    SnapshotSummaryCache, parse_search_query_for,
};

use super::{StartupFormatArg, api};

const TOKEN_BYTES: usize = 32;
const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_COUNT: usize = 64;
const MAX_HEADER_BYTES: usize = 32 * 1024;

enum InspectAuth {
    Human,
    Bearer {
        canonical_host: String,
        token: SecretToken,
    },
}

struct SecretToken(String);

impl SecretToken {
    fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes)?;
        Ok(Self(URL_SAFE_NO_PAD.encode(bytes)))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken([redacted])")
    }
}

impl fmt::Display for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

struct RequestHead {
    method: String,
    target: String,
    hosts: Vec<String>,
    authorizations: Vec<String>,
}

#[derive(Debug)]
enum RequestParseError {
    Io(std::io::Error),
    BadRequest,
    HeaderTooLarge,
}

impl From<std::io::Error> for RequestParseError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Shared, read-only inspector server state. Holds the resolved store path and the read-time
/// highlight cache; cloned cheaply behind an `Arc` to every connection thread.
pub(super) struct InspectState {
    pub repo: PathBuf,
    pub highlight_cache: RwLock<HighlightCache>,
    /// The single-slot base-projection cache (#255): repeated `/api/history`
    /// queries over one store version reuse the fully-hydrated base instead of
    /// re-reading and re-hydrating the whole log per request (INV-5).
    pub history_cache: super::cache::HistoryProjectionCache,
    /// The single-slot `/api/revisions` response cache (#426): one payload per
    /// store version, rebuilt only when the head marker moves.
    pub revisions_cache: super::cache::RevisionsResponseCache,
    /// Content-hash-keyed snapshot summary counts shared across every
    /// `/api/revisions` rebuild (#426): each snapshot artifact is decoded once
    /// per server process, not once per rebuild.
    pub snapshot_summaries: Arc<SnapshotSummaryCache>,
    /// Dedup flag for the background `/api/revisions` rewarm: at most one warm
    /// thread runs at a time, no matter how many freshness polls observe a
    /// moved marker.
    revisions_warm_in_flight: AtomicBool,
}

impl InspectState {
    pub(super) fn new(repo: PathBuf) -> Self {
        Self {
            repo,
            highlight_cache: RwLock::new(HighlightCache::new(HIGHLIGHT_CACHE_CAPACITY)),
            history_cache: super::cache::HistoryProjectionCache::new(),
            revisions_cache: super::cache::RevisionsResponseCache::new(),
            snapshot_summaries: Arc::new(SnapshotSummaryCache::new()),
            revisions_warm_in_flight: AtomicBool::new(false),
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
const POINTBREAK_LOGO_MONO_SVG: &[u8] = include_bytes!("assets/pointbreak-logo-mono.svg");
const FAVICON_SVG: &[u8] = include_bytes!("assets/favicon.svg");
const FAVICON_PNG: &[u8] = include_bytes!("assets/favicon.png");
const FAVICON_DARK_PNG: &[u8] = include_bytes!("assets/favicon-dark.png");

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

    fn asset_bytes(content_type: &'static str, body: &[u8]) -> Self {
        Self::new("200 OK", content_type, body.to_vec())
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

    fn unauthorized() -> Self {
        Self::new("401 Unauthorized", "text/plain; charset=utf-8", Vec::new())
    }
}

pub(super) fn serve(
    addr: SocketAddr,
    repo: PathBuf,
    open: bool,
    startup_format: StartupFormatArg,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener =
        TcpListener::bind(addr).map_err(|error| format!("could not bind {addr}: {error}"))?;
    // Resolve the actually-bound address so an ephemeral port (`--port 0`)
    // is shown and opened correctly rather than `:0`.
    let bound = listener.local_addr().unwrap_or(addr);
    let url = format!("http://{bound}/");
    let auth = Arc::new(match startup_format {
        StartupFormatArg::Human => InspectAuth::Human,
        StartupFormatArg::Json => InspectAuth::Bearer {
            canonical_host: bound.to_string(),
            token: SecretToken::generate()
                .map_err(|error| format!("could not generate inspect bearer: {error}"))?,
        },
    });

    match auth.as_ref() {
        InspectAuth::Human => {
            writeln!(stdout, "Pointbreak Review inspector")?;
            writeln!(stdout, "  store: {}", repo.display())?;
            writeln!(stdout, "  url:   {url}")?;
            writeln!(stdout, "  stop:  Ctrl-C")?;
        }
        InspectAuth::Bearer { token, .. } => {
            serde_json::to_writer(
                &mut *stdout,
                &InspectStartupDocument::new(bound.ip().to_string(), bound.port(), token.expose()),
            )?;
            writeln!(stdout)?;
        }
    }
    stdout.flush().ok();

    let state = Arc::new(InspectState::new(repo));
    warm_caches(Arc::clone(&state));

    if open && matches!(auth.as_ref(), InspectAuth::Human) {
        open_browser(&url);
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                let auth = Arc::clone(&auth);
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &state, &auth) {
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

/// Warm both store-version caches in the background at startup: history first
/// (the timeline is the first paint), then the `/api/revisions` payload (the
/// expensive one — every revision's overview). Best-effort: endpoint requests
/// use the same cache paths and surface any real error to the client.
fn warm_caches(state: Arc<InspectState>) {
    thread::spawn(move || {
        if let Err(error) = api::warm_history_cache(state.repo.as_path(), &state.history_cache) {
            tracing::debug!(error = %error, "inspect_history_cache_warm_failed");
        } else {
            tracing::debug!("inspect_history_cache_warmed");
        }
        if let Err(error) = api::warm_revisions_cache(
            state.repo.as_path(),
            &state.revisions_cache,
            &state.snapshot_summaries,
        ) {
            tracing::debug!(error = %error, "inspect_revisions_cache_warm_failed");
        } else {
            tracing::debug!("inspect_revisions_cache_warmed");
        }
    });
}

/// Kick a deduped background rebuild of the `/api/revisions` payload when the
/// freshness poll observes a store version the cache does not hold. The poll
/// is the client's own change detector, so by the time it refetches
/// `/api/revisions` the rebuild has usually already started (or finished)
/// instead of blocking that request for the full build.
fn maybe_warm_revisions_cache(state: &Arc<InspectState>, commit_graph_stamp: Option<&str>) {
    if api::revisions_cache_is_warm(
        state.repo.as_path(),
        &state.revisions_cache,
        commit_graph_stamp,
    ) {
        return;
    }
    if state.revisions_warm_in_flight.swap(true, Ordering::AcqRel) {
        return;
    }
    let state = Arc::clone(state);
    thread::spawn(move || {
        if let Err(error) = api::warm_revisions_cache(
            state.repo.as_path(),
            &state.revisions_cache,
            &state.snapshot_summaries,
        ) {
            tracing::debug!(error = %error, "inspect_revisions_cache_rewarm_failed");
        }
        state
            .revisions_warm_in_flight
            .store(false, Ordering::Release);
    });
}

fn handle_connection(
    stream: TcpStream,
    state: &Arc<InspectState>,
    auth: &InspectAuth,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;

    let read_half = stream.try_clone()?;
    let mut reader = BufReader::new(read_half);

    let request = match parse_request_head(&mut reader) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(RequestParseError::Io(error)) => return Err(error),
        Err(RequestParseError::BadRequest) => {
            return write_response(stream, &Response::text("400 Bad Request", "bad request"));
        }
        Err(RequestParseError::HeaderTooLarge) => {
            return write_response(
                stream,
                &Response::text(
                    "431 Request Header Fields Too Large",
                    "request headers too large",
                ),
            );
        }
    };

    if !is_authorized(auth, &request) {
        return write_response(stream, &Response::unauthorized());
    }

    let (path, query) = split_target(&request.target);
    let response = route(state, &request.method, path, query);
    write_response(stream, &response)
}

fn parse_request_head(reader: &mut impl BufRead) -> Result<Option<RequestHead>, RequestParseError> {
    let Some(request_line) = read_bounded_line(reader, MAX_REQUEST_LINE_BYTES)? else {
        return Ok(None);
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(RequestParseError::BadRequest)?;
    let target = parts.next().ok_or(RequestParseError::BadRequest)?;
    let protocol = parts.next().ok_or(RequestParseError::BadRequest)?;
    if parts.next().is_some() || !protocol.starts_with("HTTP/1.") {
        return Err(RequestParseError::BadRequest);
    }

    let mut header_count = 0_usize;
    let mut header_bytes = 0_usize;
    let mut hosts = Vec::new();
    let mut authorizations = Vec::new();
    loop {
        let Some(line) = read_bounded_line(reader, MAX_HEADER_BYTES)? else {
            return Err(RequestParseError::BadRequest);
        };
        header_bytes = header_bytes.saturating_add(line.len());
        if header_bytes > MAX_HEADER_BYTES {
            return Err(RequestParseError::HeaderTooLarge);
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        header_count += 1;
        if header_count > MAX_HEADER_COUNT {
            return Err(RequestParseError::HeaderTooLarge);
        }

        let line = line.trim_end_matches(['\r', '\n']);
        let (name, value) = line.split_once(':').ok_or(RequestParseError::BadRequest)?;
        let value = value.trim_matches([' ', '\t']);
        if name.eq_ignore_ascii_case("host") {
            hosts.push(value.to_owned());
        } else if name.eq_ignore_ascii_case("authorization") {
            authorizations.push(value.to_owned());
        }
    }

    Ok(Some(RequestHead {
        method: method.to_owned(),
        target: target.to_owned(),
        hosts,
        authorizations,
    }))
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    limit: usize,
) -> Result<Option<String>, RequestParseError> {
    let mut bytes = Vec::new();
    let read = reader
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > limit || !bytes.ends_with(b"\n") {
        return Err(RequestParseError::HeaderTooLarge);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| RequestParseError::BadRequest)
}

fn is_authorized(auth: &InspectAuth, request: &RequestHead) -> bool {
    let InspectAuth::Bearer {
        canonical_host,
        token,
    } = auth
    else {
        return true;
    };

    let [host] = request.hosts.as_slice() else {
        return false;
    };
    let [authorization] = request.authorizations.as_slice() else {
        return false;
    };
    if host != canonical_host {
        return false;
    }
    let Some(presented) = authorization.strip_prefix("Bearer ") else {
        return false;
    };
    secret_eq(presented.as_bytes(), token.expose().as_bytes())
}

fn secret_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn route(state: &Arc<InspectState>, method: &str, path: &str, query: Option<&str>) -> Response {
    if method != "GET" {
        return Response::text("405 Method Not Allowed", "method not allowed");
    }

    let repo = state.repo.as_path();
    match path {
        "/" | "/index.html" => Response::asset("text/html; charset=utf-8", INDEX_HTML),
        "/tokens.css" => Response::asset("text/css; charset=utf-8", TOKENS_CSS),
        "/app.css" => Response::asset("text/css; charset=utf-8", APP_CSS),
        "/app.js" => Response::asset("application/javascript; charset=utf-8", APP_JS),
        "/pointbreak-logo-mono.svg" => {
            Response::asset_bytes("image/svg+xml; charset=utf-8", POINTBREAK_LOGO_MONO_SVG)
        }
        "/favicon.svg" => Response::asset_bytes("image/svg+xml; charset=utf-8", FAVICON_SVG),
        "/favicon.png" => Response::asset_bytes("image/png", FAVICON_PNG),
        "/favicon-dark.png" => Response::asset_bytes("image/png", FAVICON_DARK_PNG),
        // The poll probe shares `/api/history` filtering but returns no entries.
        "/api/history/new-count" => match history_query(query) {
            Ok(request) => {
                let since_occurred_at = query_param(query, "sinceOccurredAt");
                let since_event_id = query_param(query, "sinceEventId");
                match (since_occurred_at, since_event_id) {
                    (None, None) => api_response(api::zero_new_count_json()),
                    (Some(occurred_at), Some(event_id)) => api_response(api::new_count_json(
                        repo,
                        &state.history_cache,
                        &request.query,
                        &occurred_at,
                        &event_id,
                    )),
                    _ => Response::json_error("400 Bad Request", "incomplete history cursor"),
                }
            }
            Err(message) => Response::json_error("400 Bad Request", &message),
        },
        "/api/history" => match history_query(query) {
            Ok(request) => api_response(api::history_json(
                repo,
                &state.history_cache,
                &request.query,
                &request.page,
            )),
            Err(message) => Response::json_error("400 Bad Request", &message),
        },
        "/api/revisions" => api_response(api::revisions_json(
            repo,
            &state.revisions_cache,
            &state.snapshot_summaries,
        )),
        "/api/threads" => api_response(api::threads_json(repo)),
        "/api/attention" => {
            // An empty `revision=` is absent, matching the exact-match history
            // params (`track=`/`snapshot=`).
            let revision = query_param(query, "revision").filter(|value| !value.is_empty());
            api_response(api::attention_json(repo, revision.as_deref()))
        }
        "/api/freshness" => {
            // The freshness poll is the client's change detector; ride it to
            // start rebuilding the expensive revisions payload before the
            // client's follow-up refetch arrives. The commit-graph stamp is
            // derived ONCE per poll (two git spawns) and shared by the warm
            // gate and the payload.
            let stamp = api::freshness_commit_graph_stamp(repo);
            maybe_warm_revisions_cache(state, stamp.as_deref());
            api_response(api::freshness_json(repo, stamp))
        }
        "/api/version" => api_response(
            serde_json::to_string(&version_document()).map_err(|error| error.to_string()),
        ),
        "/api/identity" => api_response(api::identity_json(repo)),
        "/favicon.ico" => Response::new("204 No Content", "image/x-icon", Vec::new()),
        _ => route_member(state, path, query),
    }
}

/// Path-member routes: `/api/revisions/{id}` and `/api/snapshots/{id}`. An empty
/// member (a trailing slash with no id) is a `400`; anything else unmatched is a
/// `404`. The id segment arrives percent-encoded (the client encodes it with
/// `encodeURIComponent`) and is decoded here.
fn route_member(state: &Arc<InspectState>, path: &str, query: Option<&str>) -> Response {
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
/// `q` is free text; `track`/`snapshot`/`at` are exact (empty => absent); `type` is
/// a comma-separated enabled-type set (absent => all types); `order` is
/// `asc`/`desc` (absent/empty => asc). A non-numeric `limit`/`offset` or an unknown
/// `order` is a usage error the caller turns into a `400` without touching the
/// store. The `at` › `offset` precedence lives in `apply_history_query`; the parser
/// only collects the params. Paging is positional (`offset`/`at`); the opaque
/// forward cursor stays on the CLI path (`shore history --cursor`). The
/// legacy `object=` param aliases to `snapshot=` for old bookmarks (#334).
fn history_query(query: Option<&str>) -> Result<HistoryRequest, String> {
    let q = query_param(query, "q").unwrap_or_default();
    // A known-but-unsupported qualifier or out-of-set value in `q` is a usage
    // error (400), never a silently-empty page; a deprecation hint is not fatal
    // and rides back on `queryNotices`.
    let parsed = parse_search_query_for(&q, QuerySurface::Event);
    if let Some(fatal) = parsed.diagnostics.iter().find(|d| {
        matches!(
            d.code,
            QueryDiagnosticCode::UnsupportedQualifier | QueryDiagnosticCode::UnsupportedValue
        )
    }) {
        return Err(fatal.message.clone());
    }
    let track = query_param(query, "track").filter(|value| !value.is_empty());
    let snapshot = query_param(query, "snapshot")
        .or_else(|| query_param(query, "object"))
        .filter(|value| !value.is_empty());
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
            snapshot,
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
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
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
    use std::io::Cursor;

    use super::*;

    fn route_for(method: &str, path: &str) -> Response {
        // Static assets, 404, 405, and the missing-id snapshot case do not
        // touch the store, so the repo path is never read for these cases.
        let state = Arc::new(InspectState::new(PathBuf::from(
            "/inspect-routing-test-unused",
        )));
        route(&state, method, path, None)
    }

    fn parse(raw: impl AsRef<[u8]>) -> Result<Option<RequestHead>, RequestParseError> {
        parse_request_head(&mut Cursor::new(raw.as_ref()))
    }

    #[test]
    fn request_parser_collects_only_authentication_headers() {
        let request = parse(
            b"GET /api/version HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nX-Ignored: value\r\nAuthorization: Bearer opaque\r\n\r\n",
        )
        .expect("valid request")
        .expect("request head");

        assert_eq!(request.method, "GET");
        assert_eq!(request.target, "/api/version");
        assert_eq!(request.hosts, ["127.0.0.1:1234"]);
        assert_eq!(request.authorizations, ["Bearer opaque"]);
    }

    #[test]
    fn request_parser_rejects_excess_header_count() {
        let mut raw = String::from("GET / HTTP/1.1\r\n");
        for _ in 0..=MAX_HEADER_COUNT {
            raw.push_str("X-Test: value\r\n");
        }
        raw.push_str("\r\n");

        assert!(matches!(parse(raw), Err(RequestParseError::HeaderTooLarge)));
    }

    #[test]
    fn request_parser_rejects_excess_header_bytes() {
        let raw = format!(
            "GET / HTTP/1.1\r\nX-Test: {}\r\n\r\n",
            "a".repeat(MAX_HEADER_BYTES)
        );

        assert!(matches!(parse(raw), Err(RequestParseError::HeaderTooLarge)));
    }

    #[test]
    fn request_parser_rejects_excess_request_line_bytes() {
        let raw = format!(
            "GET /{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "a".repeat(MAX_REQUEST_LINE_BYTES)
        );

        assert!(matches!(parse(raw), Err(RequestParseError::HeaderTooLarge)));
    }

    #[test]
    fn secret_token_debug_and_display_are_redacted() {
        let token = SecretToken::generate().expect("generate token");
        assert!(!format!("{token:?}").contains(token.expose()));
        assert!(!format!("{token}").contains(token.expose()));
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
    fn history_query_reads_snapshot_param_and_aliases_legacy_object() {
        let from_new = history_query(Some("snapshot=obj-1")).unwrap();
        assert_eq!(from_new.query.snapshot.as_deref(), Some("obj-1"));
        // A stale bookmark's legacy `object=` param still resolves to snapshot (#334).
        let from_legacy = history_query(Some("object=obj-1")).unwrap();
        assert_eq!(from_legacy.query.snapshot.as_deref(), Some("obj-1"));
        // Absent => no snapshot constraint.
        let absent = history_query(Some("q=hello")).unwrap();
        assert_eq!(absent.query.snapshot, None);
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
    fn identity_route_is_registered() {
        // The path is routed to the identity builder (not a 404). Against the unused
        // test path the store resolve fails, so it is a JSON 500 — but crucially NOT
        // "404 Not Found", which is what an unrouted path returns.
        let response = route_for("GET", "/api/identity");
        assert_ne!(response.status, "404 Not Found");
        assert!(response.content_type.starts_with("application/json"));
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

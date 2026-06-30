use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::Args;

mod api;
mod server;
// The enriched wire DTO lands before its consumer (the snapshot read seam), so its items are
// briefly unused outside tests. The allow is removed once that seam builds the DTO.
#[allow(dead_code)]
mod wire;

/// `shore inspect` starts a small local web server that visualizes a `.shore/data`
/// store: the event timeline, captured Revisions, and recorded outcomes.
///
/// The server is intentionally synchronous (thread-per-connection, std only).
/// It introduces no async runtime, matching the storage-model guidance, and
/// reuses the same validated projections as `shore review history` /
/// `shore review unit list`, so it never parses raw `.shore/data/` files itself.
#[derive(Debug, Args)]
pub(super) struct InspectArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Address to bind the inspector server to.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind the inspector server to.
    #[arg(long, default_value_t = 7878)]
    port: u16,

    /// Open the inspector in the default browser after the server starts.
    #[arg(long)]
    open: bool,
}

pub(super) fn run(
    args: InspectArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let span = tracing::info_span!("shore.inspect");
    let _entered = span.enter();
    tracing::debug!(command = "inspect", "command_start");

    let ip: IpAddr = args
        .host
        .parse()
        .map_err(|_| format!("invalid --host value: {}", args.host))?;
    let addr = SocketAddr::new(ip, args.port);
    server::serve(addr, args.repo, args.open, stdout)
}

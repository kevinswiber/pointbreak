use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::{Args, ValueEnum};

mod api;
mod cache;
mod server;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum StartupFormatArg {
    Human,
    Json,
}

/// `shore inspect` starts a small local web server that visualizes a `.shore/data`
/// store: the event timeline, captured Revisions, and recorded outcomes.
///
/// The server is intentionally synchronous (thread-per-connection, std only).
/// It introduces no async runtime, matching the storage-model guidance, and
/// reuses the same validated projections as `shore history` /
/// `shore revision list`, so it never parses raw `.shore/data/` files itself.
#[derive(Debug, Args)]
pub(super) struct InspectArgs {
    /// Repository root or a path inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Loopback IP address to bind the inspector server to.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind the inspector server to.
    #[arg(long, default_value_t = 7878)]
    port: u16,

    /// Open the inspector in the default browser after the server starts.
    #[arg(long)]
    open: bool,

    /// Startup output and request-authentication mode.
    #[arg(long, value_enum, default_value_t = StartupFormatArg::Human)]
    startup_format: StartupFormatArg,
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
    if !ip.is_loopback() {
        return Err(format!("--host must be a loopback IP address: {ip}").into());
    }
    if args.startup_format == StartupFormatArg::Json && args.open {
        return Err("--open cannot be used with --startup-format json".into());
    }
    let addr = SocketAddr::new(ip, args.port);
    server::serve(addr, args.repo, args.open, args.startup_format, stdout)
}

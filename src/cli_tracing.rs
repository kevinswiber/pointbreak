use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::Mutex;

use clap::{Args, ValueEnum};
use shore::perf::{self, PERF_TARGET, PerfLayer};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Clone, Debug, Args)]
pub(crate) struct TracingArgs {
    #[arg(long, global = true, value_name = "FILTER")]
    pub(crate) log: Option<String>,

    #[arg(long, global = true, value_enum, default_value_t = LogFormatArg::Compact)]
    pub(crate) log_format: LogFormatArg,

    #[arg(long, global = true, value_name = "PATH")]
    pub(crate) log_file: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum LogFormatArg {
    Compact,
    Pretty,
    Json,
}

pub(crate) fn tracing_enabled(args: &TracingArgs) -> bool {
    resolve_log_filter(args).is_some() || perf::is_enabled()
}

pub(crate) fn init_tracing(args: &TracingArgs) -> Result<(), Box<dyn std::error::Error>> {
    let perf_enabled = perf::is_enabled();
    let log_filter = resolve_log_filter(args);
    if log_filter.is_none() && !perf_enabled {
        return Ok(());
    }

    let filter_str = compose_filter(log_filter.as_deref(), perf_enabled);
    let filter = EnvFilter::try_new(&filter_str)
        .map_err(|error| invalid_input(format!("invalid log filter: {error}")))?;
    let (writer, ansi) = writer(args.log_file.as_ref())?;

    init_tracing_with_writer(filter, args.log_format, writer, ansi, perf_enabled)
}

pub(crate) fn init_tracing_with_writer(
    filter: EnvFilter,
    format: LogFormatArg,
    writer: BoxMakeWriter,
    ansi: bool,
    perf_enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let perf_layer = perf_enabled.then(PerfLayer::new);
    let registry = tracing_subscriber::registry().with(filter).with(perf_layer);
    let fmt_base = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(ansi);

    match format {
        LogFormatArg::Compact => registry.with(fmt_base.compact()).try_init(),
        LogFormatArg::Pretty => registry.with(fmt_base.pretty()).try_init(),
        LogFormatArg::Json => registry.with(fmt_base.json()).try_init(),
    }
    .map_err(|error| io::Error::other(error.to_string()))?;

    Ok(())
}

fn compose_filter(log: Option<&str>, perf_enabled: bool) -> String {
    let perf_directives = format!("{PERF_TARGET}=info,shore=debug");
    match (log, perf_enabled) {
        (Some(filter), true) => format!("{filter},{perf_directives}"),
        (Some(filter), false) => filter.to_owned(),
        (None, true) => format!("off,{perf_directives}"),
        (None, false) => "off".to_owned(),
    }
}

fn resolve_log_filter(args: &TracingArgs) -> Option<String> {
    if let Some(filter) = args.log.as_deref() {
        return active_filter(filter);
    }

    if let Ok(filter) = std::env::var("SHORE_LOG") {
        if is_off(&filter) {
            return None;
        }
        if let Some(filter) = active_filter(&filter) {
            return Some(filter);
        }
    }

    std::env::var("RUST_LOG")
        .ok()
        .and_then(|filter| active_filter(&filter))
}

fn active_filter(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("off") {
        None
    } else {
        Some(value.to_owned())
    }
}

fn is_off(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("off")
}

fn writer(log_file: Option<&PathBuf>) -> io::Result<(BoxMakeWriter, bool)> {
    match log_file {
        Some(path) => {
            let file = append_file(path)?;
            Ok((BoxMakeWriter::new(Mutex::new(file)), false))
        }
        None => Ok((BoxMakeWriter::new(io::stderr), io::stderr().is_terminal())),
    }
}

fn append_file(path: &PathBuf) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    OpenOptions::new().create(true).append(true).open(path)
}

fn invalid_input(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_filter_returns_off_when_nothing_enabled() {
        assert_eq!(compose_filter(None, false), "off");
    }

    #[test]
    fn compose_filter_passes_user_filter_through_when_perf_disabled() {
        assert_eq!(compose_filter(Some("shore=info"), false), "shore=info");
    }

    #[test]
    fn compose_filter_enables_perf_directives_when_only_perf_set() {
        let composed = compose_filter(None, true);
        assert!(composed.starts_with("off,"), "got: {composed}");
        assert!(composed.contains(PERF_TARGET));
        assert!(composed.contains("shore=debug"));
        // EnvFilter should accept the composed directives.
        EnvFilter::try_new(&composed).expect("composed filter parses");
    }

    #[test]
    fn compose_filter_merges_user_filter_with_perf_directives() {
        let composed = compose_filter(Some("warn"), true);
        assert!(composed.starts_with("warn,"));
        assert!(composed.contains(PERF_TARGET));
        EnvFilter::try_new(&composed).expect("composed filter parses");
    }
}

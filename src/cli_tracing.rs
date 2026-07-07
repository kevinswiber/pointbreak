use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::Mutex;

use clap::{Args, ValueEnum};
use pointbreak::perf::{self, PERF_TARGET, PerfLayer};
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Per-layer filter applied to `PerfLayer` so it observes shore spans
/// (including the existing `event_store.*` debug spans) regardless of the
/// user's `--log` filter, without bleeding those events into the fmt layer.
const PERF_OBSERVE_FILTER: &str = "shore=debug";

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

pub(crate) fn init_tracing(args: &TracingArgs) -> Result<(), Box<dyn std::error::Error>> {
    let perf_enabled = perf::is_enabled();
    let log_filter = resolve_log_filter(args);
    if log_filter.is_none() && !perf_enabled {
        return Ok(());
    }

    let fmt_filter_str = compose_fmt_filter(log_filter.as_deref(), perf_enabled);
    let fmt_filter = EnvFilter::try_new(&fmt_filter_str)
        .map_err(|error| invalid_input(format!("invalid log filter: {error}")))?;
    let (writer, ansi) = writer(args.log_file.as_ref())?;

    init_tracing_with_writer(fmt_filter, args.log_format, writer, ansi, perf_enabled)
}

pub(crate) fn init_tracing_with_writer(
    fmt_filter: EnvFilter,
    format: LogFormatArg,
    writer: BoxMakeWriter,
    ansi: bool,
    perf_enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let perf_layer = perf_enabled
        .then(|| {
            EnvFilter::try_new(PERF_OBSERVE_FILTER)
                .map(|filter| PerfLayer::new().with_filter(filter))
        })
        .transpose()
        .map_err(|error| invalid_input(format!("invalid perf observe filter: {error}")))?;

    let registry = tracing_subscriber::registry().with(perf_layer);
    let fmt_base = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(ansi);

    match format {
        LogFormatArg::Compact => registry
            .with(fmt_base.compact().with_filter(fmt_filter))
            .try_init(),
        LogFormatArg::Pretty => registry
            .with(fmt_base.pretty().with_filter(fmt_filter))
            .try_init(),
        LogFormatArg::Json => registry
            .with(fmt_base.json().with_filter(fmt_filter))
            .try_init(),
    }
    .map_err(|error| io::Error::other(error.to_string()))?;

    Ok(())
}

fn compose_fmt_filter(log: Option<&str>, perf_enabled: bool) -> String {
    match (log, perf_enabled) {
        (Some(filter), true) => format!("{filter},{PERF_TARGET}=info"),
        (Some(filter), false) => filter.to_owned(),
        (None, true) => format!("off,{PERF_TARGET}=info"),
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
    fn compose_fmt_filter_returns_off_when_nothing_enabled() {
        assert_eq!(compose_fmt_filter(None, false), "off");
    }

    #[test]
    fn compose_fmt_filter_passes_user_filter_through_when_perf_disabled() {
        assert_eq!(compose_fmt_filter(Some("shore=info"), false), "shore=info");
    }

    #[test]
    fn compose_fmt_filter_only_lets_perf_events_through_when_perf_alone_is_set() {
        let composed = compose_fmt_filter(None, true);
        assert_eq!(composed, format!("off,{PERF_TARGET}=info"));
        assert!(
            !composed.contains("shore=debug"),
            "fmt filter must not enable broad shore debug output: {composed}"
        );
        EnvFilter::try_new(&composed).expect("composed filter parses");
    }

    #[test]
    fn compose_fmt_filter_merges_user_filter_with_perf_target_only() {
        let composed = compose_fmt_filter(Some("warn"), true);
        assert_eq!(composed, format!("warn,{PERF_TARGET}=info"));
        EnvFilter::try_new(&composed).expect("composed filter parses");
    }

    #[test]
    fn perf_observe_filter_is_valid() {
        EnvFilter::try_new(PERF_OBSERVE_FILTER).expect("perf observe filter parses");
    }
}

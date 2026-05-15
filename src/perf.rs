//! Opt-in tracing layer that records per-span elapsed time and emits one
//! structured event per root span on close.
//!
//! The layer is intended for ad-hoc diagnostics of the review pipeline. It
//! attaches a start `Instant` to every span via `on_new_span`, accumulates each
//! child's elapsed time on the parent's extensions as the child closes, and
//! when a root span closes emits a single `tracing::info!` event with a JSON
//! `steps` field carrying the nested timing tree.
//!
//! Activation is gated by the `SHORE_PERF` environment variable. The layer is
//! only installed when the variable is set to a truthy value; see
//! `cli_tracing::init_tracing` for the wiring.

use std::time::Instant;

use serde::Serialize;
use tracing::span::Attributes;
use tracing::{Id, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// `target` used for the per-root perf event emitted by [`PerfLayer`].
pub const PERF_TARGET: &str = "shore::perf";
/// Static message used for the per-root perf event.
pub const PERF_ROOT_EVENT: &str = "perf_root";

/// Environment variable that enables the perf layer when set to a truthy
/// value (`1`, `true`, case-insensitive).
pub const PERF_ENV_VAR: &str = "SHORE_PERF";

/// Returns `true` when the perf layer should be installed by the CLI.
pub fn is_enabled() -> bool {
    std::env::var(PERF_ENV_VAR)
        .ok()
        .map(|value| is_truthy(&value))
        .unwrap_or(false)
}

fn is_truthy(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && !value.eq_ignore_ascii_case("0") && !value.eq_ignore_ascii_case("false")
}

#[derive(Debug)]
struct PerfSpanState {
    start: Instant,
    children: Vec<TimingNode>,
}

/// One node in the nested timing tree emitted by [`PerfLayer`].
#[derive(Debug, Serialize)]
pub struct TimingNode {
    pub name: String,
    pub elapsed_us: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TimingNode>,
}

/// Tracing layer that emits per-root perf events.
#[derive(Debug, Default)]
pub struct PerfLayer;

impl PerfLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for PerfLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(PerfSpanState {
                start: Instant::now(),
                children: Vec::new(),
            });
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let Some(state) = span.extensions_mut().remove::<PerfSpanState>() else {
            return;
        };
        let elapsed_us = u64::try_from(state.start.elapsed().as_micros()).unwrap_or(u64::MAX);
        let node = TimingNode {
            name: span.metadata().name().to_owned(),
            elapsed_us,
            children: state.children,
        };

        if let Some(parent) = span.parent() {
            let mut extensions = parent.extensions_mut();
            if let Some(parent_state) = extensions.get_mut::<PerfSpanState>() {
                parent_state.children.push(node);
                return;
            }
        }

        emit_root_event(&node);
    }
}

fn emit_root_event(node: &TimingNode) {
    let steps = serde_json::to_string(&node.children).unwrap_or_else(|_| "[]".to_owned());
    tracing::info!(
        target: PERF_TARGET,
        root = %node.name,
        elapsed_us = node.elapsed_us,
        steps = %steps,
        "{PERF_ROOT_EVENT}",
    );
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::thread::sleep;
    use std::time::Duration;

    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::LookupSpan;

    use super::*;

    #[derive(Clone, Debug)]
    struct CapturedEvent {
        target: String,
        fields: BTreeMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct CaptureLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl CaptureLayer {
        fn snapshot(&self) -> Vec<CapturedEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(CapturedEvent {
                target: event.metadata().target().to_owned(),
                fields: visitor.0,
            });
        }
    }

    #[derive(Default)]
    struct FieldVisitor(BTreeMap<String, String>);

    impl Visit for FieldVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.0.insert(field.name().to_owned(), value.to_string());
        }
    }

    fn perf_events(capture: &CaptureLayer) -> Vec<CapturedEvent> {
        capture
            .snapshot()
            .into_iter()
            .filter(|event| event.target == PERF_TARGET)
            .collect()
    }

    fn with_perf_subscriber<F: FnOnce()>(capture: CaptureLayer, body: F) {
        let subscriber = tracing_subscriber::registry()
            .with(PerfLayer::new())
            .with(capture);
        tracing::subscriber::with_default(subscriber, body);
    }

    #[test]
    fn root_span_close_emits_perf_event_with_child_timings() {
        let capture = CaptureLayer::default();
        with_perf_subscriber(capture.clone(), || {
            let root = tracing::info_span!("shore.test.root");
            let _guard = root.enter();
            {
                let child = tracing::info_span!("shore.test.child");
                let _guard = child.enter();
                sleep(Duration::from_millis(1));
            }
        });

        let events = perf_events(&capture);
        assert_eq!(events.len(), 1, "expected exactly one perf event");
        let event = &events[0];
        assert_eq!(
            event.fields.get("root").map(String::as_str),
            Some("shore.test.root")
        );
        assert_eq!(
            event.fields.get("message").map(String::as_str),
            Some(PERF_ROOT_EVENT)
        );
        let elapsed_us: u64 = event
            .fields
            .get("elapsed_us")
            .expect("elapsed_us field")
            .parse()
            .expect("elapsed_us parses as u64");
        assert!(elapsed_us > 0, "expected nonzero elapsed_us");
        let steps = event.fields.get("steps").expect("steps field");
        let parsed: serde_json::Value =
            serde_json::from_str(steps).expect("steps field is valid JSON");
        let array = parsed.as_array().expect("steps is a JSON array");
        assert_eq!(array.len(), 1);
        assert_eq!(array[0]["name"], "shore.test.child");
        assert!(array[0]["elapsed_us"].as_u64().unwrap_or(0) > 0);
    }

    #[test]
    fn multiple_siblings_accumulate_under_parent() {
        let capture = CaptureLayer::default();
        with_perf_subscriber(capture.clone(), || {
            let root = tracing::info_span!("shore.test.root");
            let _guard = root.enter();
            for _ in 0..3 {
                let child = tracing::info_span!("shore.test.phase");
                let _guard = child.enter();
            }
        });

        let events = perf_events(&capture);
        assert_eq!(events.len(), 1);
        let steps = events[0].fields.get("steps").expect("steps field");
        let parsed: serde_json::Value = serde_json::from_str(steps).expect("steps is JSON");
        let array = parsed.as_array().expect("steps is array");
        assert_eq!(array.len(), 3);
        for child in array {
            assert_eq!(child["name"], "shore.test.phase");
        }
    }

    #[test]
    fn independent_root_spans_produce_independent_events() {
        let capture = CaptureLayer::default();
        with_perf_subscriber(capture.clone(), || {
            {
                let span = tracing::info_span!("shore.test.first");
                let _guard = span.enter();
            }
            {
                let span = tracing::info_span!("shore.test.second");
                let _guard = span.enter();
            }
        });

        let events = perf_events(&capture);
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].fields.get("root").map(String::as_str),
            Some("shore.test.first")
        );
        assert_eq!(
            events[1].fields.get("root").map(String::as_str),
            Some("shore.test.second")
        );
    }

    #[test]
    fn root_span_without_children_emits_empty_steps_array() {
        let capture = CaptureLayer::default();
        with_perf_subscriber(capture.clone(), || {
            let span = tracing::info_span!("shore.test.solo");
            let _guard = span.enter();
        });

        let events = perf_events(&capture);
        assert_eq!(events.len(), 1);
        let steps = events[0].fields.get("steps").expect("steps field");
        assert_eq!(steps, "[]");
    }

    #[test]
    fn is_truthy_recognises_common_forms() {
        assert!(is_truthy("1"));
        assert!(is_truthy("true"));
        assert!(is_truthy("TRUE"));
        assert!(is_truthy("yes"));
        assert!(!is_truthy(""));
        assert!(!is_truthy("0"));
        assert!(!is_truthy("false"));
        assert!(!is_truthy("FALSE"));
    }
}

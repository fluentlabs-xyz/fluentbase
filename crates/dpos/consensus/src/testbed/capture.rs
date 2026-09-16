//! A `tracing::Subscriber` that collects WARN/ERROR events for the test on the
//! current thread. `tracing-subscriber` is not a dev-dependency, and a ~50-line
//! implementation is enough to see `SafetyHalt` and `LOG ERROR` lines.
//!
//! One global subscriber serves every test in the process (tracing allows a
//! single `set_global_default`); the deterministic runner is single-threaded, so
//! an event belongs to whichever test thread it is emitted on, and each thread
//! routes to its own sink through a thread-local. Events on a thread with no sink
//! (another test module's) are dropped.

use std::{
    cell::RefCell,
    fmt::Write as _,
    sync::{Arc, Mutex, OnceLock},
};
use tracing::{
    field::{Field, Visit},
    span, Event, Level, Metadata, Subscriber,
};

#[derive(Clone, Debug)]
pub(super) struct Captured {
    pub(super) level: Level,
    /// `target: message` followed by every other field as ` key=value`.
    pub(super) text: String,
}

pub(super) type Sink = Arc<Mutex<Vec<Captured>>>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

static INSTALLED: OnceLock<bool> = OnceLock::new();

/// Install the global subscriber once per process and bind a fresh sink to this
/// thread. Returns the sink and whether capture is live (false when another
/// subscriber already owns the global slot — then the sink stays empty).
pub(super) fn install() -> (Sink, bool) {
    let live = *INSTALLED.get_or_init(|| tracing::subscriber::set_global_default(Capture).is_ok());
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    SINK.with(|s| *s.borrow_mut() = Some(sink.clone()));
    (sink, live)
}

struct Capture;

impl Subscriber for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::WARN
    }
    fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}
    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut visitor = Text::default();
        event.record(&mut visitor);
        let captured = Captured {
            level: *event.metadata().level(),
            text: format!(
                "{}: {}{}",
                event.metadata().target(),
                visitor.message,
                visitor.fields
            ),
        };
        SINK.with(|s| {
            if let Some(sink) = s.borrow().as_ref() {
                sink.lock().unwrap().push(captured);
            }
        });
    }
    fn enter(&self, _span: &span::Id) {}
    fn exit(&self, _span: &span::Id) {}
}

#[derive(Default)]
struct Text {
    message: String,
    fields: String,
}

impl Visit for Text {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
}

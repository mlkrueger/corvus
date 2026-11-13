//! JSON-lines logging to stderr ([system/10](../../../spec/system/10-config-ops.md#logging)).
//! Fields: `ts`, `level`, `target`, `msg`, the event's own fields, then span fields. stdout is never used.

use std::fmt;

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::{JsonFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields};
use tracing_subscriber::registry::LookupSpan;

use crate::clock::{Clock, SystemClock, format_rfc3339};

/// Installs the global subscriber. Safe to call more than once (later calls are ignored).
pub fn init(level: &str) {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .fmt_fields(JsonFields::new())
        .event_format(JsonLines)
        .with_writer(std::io::stderr)
        .try_init();
}

/// The Corvus log-line format.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonLines;

impl<S, N> FormatEvent<S, N> for JsonLines
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let mut obj = Map::new();
        obj.insert("ts".into(), Value::String(format_rfc3339(SystemClock.now_ms())));
        obj.insert(
            "level".into(),
            Value::String(meta.level().as_str().to_ascii_lowercase()),
        );
        obj.insert("target".into(), Value::String(meta.target().to_owned()));
        let mut v = Collect(Map::new());
        event.record(&mut v);
        obj.insert(
            "msg".into(),
            v.0.remove("message").unwrap_or(Value::String(String::new())),
        );
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                let ext = span.extensions();
                if let Some(fields) = ext.get::<FormattedFields<N>>()
                    && let Ok(Value::Object(m)) = serde_json::from_str::<Value>(fields)
                {
                    for (k, val) in m {
                        obj.entry(k).or_insert(val);
                    }
                }
            }
        }
        for (k, val) in v.0 {
            obj.insert(k, val);
        }
        let line = serde_json::to_string(&Value::Object(obj)).map_err(|_| fmt::Error)?;
        writeln!(writer, "{line}")
    }
}

struct Collect(Map<String, Value>);

impl Visit for Collect {
    fn record_str(&mut self, f: &Field, v: &str) {
        self.0.insert(f.name().into(), Value::String(v.into()));
    }
    fn record_i64(&mut self, f: &Field, v: i64) {
        self.0.insert(f.name().into(), v.into());
    }
    fn record_u64(&mut self, f: &Field, v: u64) {
        self.0.insert(f.name().into(), v.into());
    }
    fn record_bool(&mut self, f: &Field, v: bool) {
        self.0.insert(f.name().into(), v.into());
    }
    fn record_f64(&mut self, f: &Field, v: f64) {
        self.0.insert(f.name().into(), v.into());
    }
    fn record_debug(&mut self, f: &Field, v: &dyn fmt::Debug) {
        self.0.insert(f.name().into(), Value::String(format!("{v:?}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    #[test]
    fn writes_one_json_object_per_line_with_span_fields() {
        let buf = Buf::default();
        let sub = tracing_subscriber::fmt()
            .fmt_fields(JsonFields::new())
            .event_format(JsonLines)
            .with_writer(buf.clone())
            .finish();
        tracing::subscriber::with_default(sub, || {
            let span = tracing::info_span!("op", op = "migrate", agent = "a1");
            let _g = span.enter();
            tracing::info!(duration_ms = 3u64, "applied migrations");
        });
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let v: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["level"], "info");
        assert_eq!(v["msg"], "applied migrations");
        assert_eq!(v["op"], "migrate");
        assert_eq!(v["agent"], "a1");
        assert_eq!(v["duration_ms"], 3);
        assert!(v["ts"].as_str().unwrap().ends_with('Z'));
        assert!(v["target"].as_str().unwrap().starts_with("corvus_core"));
    }
}

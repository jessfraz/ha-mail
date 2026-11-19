//! Dropshot attaches raw request URIs and errors to log records. Only the fixed
//! operational vocabulary below may leave the process, including on failures.
use serde::Serialize;
use slog::{Drain, KV};
use std::{fmt, io::Write, sync::Mutex};
use uuid::Uuid;

pub fn logger() -> slog::Logger {
    with_writer(std::io::stderr())
}
pub fn with_writer(writer: impl Write + Send + 'static) -> slog::Logger {
    slog::Logger::root(SafeLog(Mutex::new(writer)), slog::o!())
}
struct SafeLog<W>(Mutex<W>);
#[derive(Default, Serialize)]
struct Fields {
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
}
#[derive(Serialize)]
struct Entry {
    #[serde(serialize_with = "serialize_timestamp")]
    timestamp: time::UtcDateTime,
    event: &'static str,
    #[serde(flatten)]
    fields: Fields,
}
// Keep the log wire format: UTC `Z`, with 0, 3, 6 or 9 fractional digits.
// time's built-in RFC3339 formatter instead trims individual trailing zeros.
fn serialize_timestamp<S: serde::Serializer>(
    timestamp: &time::UtcDateTime,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    struct Timestamp<'a>(&'a time::UtcDateTime);
    impl fmt::Display for Timestamp<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let at = self.0;
            let year = at.year();
            if (0..=9999).contains(&year) {
                write!(f, "{year:04}")?;
            } else {
                write!(f, "{year:+05}")?;
            }
            write!(
                f,
                "-{:02}-{:02}T{:02}:{:02}:{:02}",
                u8::from(at.month()),
                at.day(),
                at.hour(),
                at.minute(),
                at.second()
            )?;
            let nanos = at.nanosecond();
            if nanos == 0 {
            } else if nanos.is_multiple_of(1_000_000) {
                write!(f, ".{:03}", nanos / 1_000_000)?;
            } else if nanos.is_multiple_of(1_000) {
                write!(f, ".{:06}", nanos / 1_000)?;
            } else {
                write!(f, ".{nanos:09}")?;
            }
            f.write_str("Z")
        }
    }
    serializer.collect_str(&Timestamp(timestamp))
}
impl<W: Write + Send> Drain for SafeLog<W> {
    type Ok = ();
    type Err = slog::Never;
    fn log(&self, record: &slog::Record<'_>, values: &slog::OwnedKVList) -> Result<(), Self::Err> {
        let event = match record.msg().as_str() {
            Some("request completed") => "request_completed",
            Some("listening") => "server_started",
            Some("beginning graceful shutdown") => "server_stopping",
            Some("integration_discovery_failed") => "integration_discovery_failed",
            Some("accept error") => "accept_failed",
            _ => return Ok(()),
        };
        let mut fields = Fields::default();
        let _ = values.serialize(record, &mut fields);
        let _ = record.kv().serialize(record, &mut fields);
        if fields.method == Some("GET") && fields.status.is_some_and(|status| status < 400) {
            return Ok(());
        }
        if let Ok(mut line) = serde_json::to_vec(&Entry {
            timestamp: time::UtcDateTime::now(),
            event,
            fields,
        }) {
            line.push(b'\n');
            if let Ok(mut writer) = self.0.lock() {
                let _ = writer.write_all(&line);
            }
        }
        Ok(())
    }
}
impl slog::Serializer for Fields {
    fn emit_arguments(&mut self, key: slog::Key, value: &fmt::Arguments<'_>) -> slog::Result {
        match key.as_str() {
            "req_id" | "request_id" => self.request_id = Uuid::parse_str(&value.to_string()).ok(),
            "method" => {
                self.method = match value.to_string().as_str() {
                    "GET" => Some("GET"),
                    "POST" => Some("POST"),
                    "PATCH" => Some("PATCH"),
                    "PUT" => Some("PUT"),
                    "DELETE" => Some("DELETE"),
                    _ => None,
                }
            }
            "response_code" => {
                self.status = value
                    .to_string()
                    .parse()
                    .ok()
                    .filter(|status| (100..=599).contains(status))
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::logging::{Entry, Fields};

    #[test]
    fn log_timestamps_keep_utc_and_grouped_fractional_precision() {
        for (nanoseconds, expected) in [
            (0, "1969-12-31T23:59:59Z"),
            (100_000_000, "1969-12-31T23:59:59.100Z"),
            (120_000_000, "1969-12-31T23:59:59.120Z"),
            (123_000_000, "1969-12-31T23:59:59.123Z"),
            (123_400_000, "1969-12-31T23:59:59.123400Z"),
            (123_456_000, "1969-12-31T23:59:59.123456Z"),
            (123_456_700, "1969-12-31T23:59:59.123456700Z"),
            (123_456_789, "1969-12-31T23:59:59.123456789Z"),
            (1, "1969-12-31T23:59:59.000000001Z"),
        ] {
            let entry = Entry {
                timestamp: time::UtcDateTime::from_unix_timestamp(-1)
                    .unwrap()
                    .replace_nanosecond(nanoseconds)
                    .unwrap(),
                event: "server_started",
                fields: Fields::default(),
            };
            assert_eq!(
                serde_json::to_string(&entry).unwrap(),
                format!(r#"{{"timestamp":"{expected}","event":"server_started"}}"#)
            );
        }
    }
}

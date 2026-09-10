//! The `unknown`-aware metric cell every telemetry row is built from.
//!
//! A telemetry column needs three states, not two: a value, a measured zero,
//! and "nobody measured it". `Option` collapses the last two and serde renders
//! them as `null`, which aggregates as nothing at all -- so an absent metric
//! silently drops out of a mean instead of being named. [`Metric`] serializes
//! the absent state as the literal string `"unknown"`, which keeps the gap
//! visible in the JSONL row and in any consumer reading it.
//!
//! Reading is deliberately lossy in one direction only: a value of the wrong
//! shape for `T` becomes unmeasured rather than failing the row, because a
//! malformed metric is a gap in one column, while a dropped event is a gap in
//! the history.

use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

/// The wire value standing in for a metric nobody measured.
pub const UNKNOWN: &str = "unknown";

/// An optional measurement. `None` serializes as the string `"unknown"`; a
/// reported zero stays the number `0`, because "measured as zero" and "never
/// measured" are different facts and the aggregate must be able to tell them
/// apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Metric<T>(pub Option<T>);

impl<T> Metric<T> {
    /// A metric that was reported.
    pub fn measured(value: T) -> Self {
        Metric(Some(value))
    }

    /// A metric the provider never reported.
    pub fn unknown() -> Self {
        Metric(None)
    }

    pub fn is_unknown(&self) -> bool {
        self.0.is_none()
    }
}

impl<T: Serialize> Serialize for Metric<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Some(ref value) => value.serialize(serializer),
            None => serializer.serialize_str(UNKNOWN),
        }
    }
}

/// Reading a reported value out of a JSON scalar. A number-as-string is
/// accepted because harnesses stringify whatever they measured; a value of the
/// wrong shape counts as unmeasured rather than failing the whole row.
pub trait MetricValue: Sized {
    fn from_json(value: &Value) -> Option<Self>;
}

impl MetricValue for u64 {
    fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => whole_number(number),
            Value::String(text) => text.trim().parse::<u64>().ok(),
            _ => None,
        }
    }
}

/// A JSON number read as an unsigned integer: exact when it fits, otherwise the
/// float form only when it carries no fraction.
fn whole_number(number: &Number) -> Option<u64> {
    if let Some(value) = number.as_u64() {
        return Some(value);
    }
    number
        .as_f64()
        .filter(|value| *value >= 0.0 && value.fract() == 0.0)
        .map(|value| value as u64)
}

impl MetricValue for f64 {
    fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse::<f64>().ok(),
            _ => None,
        }
    }
}

impl MetricValue for bool {
    fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::Bool(flag) => Some(*flag),
            Value::String(text) => match text.trim() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }
}

impl MetricValue for String {
    fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        }
    }
}

/// `Metric` reads the JSON scalar verbatim: a number, a bool, a string, or
/// `"unknown"`/`null` for "nobody measured it". A value of the wrong shape for
/// `T` degrades to unknown -- a malformed metric is a gap in the record, not a
/// reason to lose the event around it.
impl<'de, T: MetricValue> Deserialize<'de> for Metric<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        Ok(match &raw {
            Value::Null => Metric(None),
            Value::String(text) if text.trim() == UNKNOWN => Metric(None),
            other => Metric(T::from_json(other)),
        })
    }
}

impl<T> From<Option<T>> for Metric<T> {
    fn from(value: Option<T>) -> Self {
        Metric(value)
    }
}

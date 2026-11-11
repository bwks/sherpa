//! Reading a row's columns in order, and turning values into the database's.

use anyhow::{Context, Result, bail};
use turso::{Row, Value};

use crate::message::TaskUsage;

/// The columns of one row, read left to right.
pub struct Cells<'a> {
    row: &'a Row,
    index: usize,
}

impl<'a> Cells<'a> {
    pub fn new(row: &'a Row) -> Self {
        Self { row, index: 0 }
    }

    fn next(&mut self, field: &str) -> Result<Value> {
        let value = self
            .row
            .get_value(self.index)
            .with_context(|| format!("No column {} ({field})", self.index))?;
        self.index += 1;
        Ok(value)
    }

    pub fn text(&mut self, field: &str) -> Result<String> {
        match self.next(field)? {
            Value::Text(text) => Ok(text),
            other => bail!("{field} is not text: {other:?}"),
        }
    }

    pub fn optional_text(&mut self, field: &str) -> Result<Option<String>> {
        match self.next(field)? {
            Value::Null => Ok(None),
            Value::Text(text) => Ok(Some(text)),
            other => bail!("{field} is not text: {other:?}"),
        }
    }

    pub fn count(&mut self, field: &str) -> Result<u64> {
        match self.next(field)? {
            Value::Integer(value) => {
                u64::try_from(value).with_context(|| format!("{field} is negative: {value}"))
            }
            other => bail!("{field} is not an integer: {other:?}"),
        }
    }

    pub fn optional_count(&mut self, field: &str) -> Result<Option<u64>> {
        match self.next(field)? {
            Value::Null => Ok(None),
            Value::Integer(value) => u64::try_from(value)
                .map(Some)
                .with_context(|| format!("{field} is negative: {value}")),
            other => bail!("{field} is not an integer: {other:?}"),
        }
    }

    pub fn optional_real(&mut self, field: &str) -> Result<Option<f64>> {
        match self.next(field)? {
            Value::Null => Ok(None),
            Value::Real(value) => Ok(Some(value)),
            other => bail!("{field} is not a number: {other:?}"),
        }
    }

    /// A task's usage from its nine columns, `runs` to `duration_api_ms`; `None` when `runs` is
    /// NULL.
    pub fn task_usage(&mut self) -> Result<Option<TaskUsage>> {
        let runs = self.optional_count("runs")?;
        let cost_usd = self.optional_real("cost_usd")?;
        let input = self.optional_count("input")?;
        let output = self.optional_count("output")?;
        let cache_creation = self.optional_count("cache_creation")?;
        let cache_read = self.optional_count("cache_read")?;
        let num_turns = self.optional_count("num_turns")?;
        let duration_ms = self.optional_count("duration_ms")?;
        let duration_api_ms = self.optional_count("duration_api_ms")?;
        Ok(runs.map(|runs| TaskUsage {
            runs,
            cost_usd: cost_usd.unwrap_or_default(),
            input: input.unwrap_or_default(),
            output: output.unwrap_or_default(),
            cache_creation: cache_creation.unwrap_or_default(),
            cache_read: cache_read.unwrap_or_default(),
            num_turns: num_turns.unwrap_or_default(),
            duration_ms: duration_ms.unwrap_or_default(),
            duration_api_ms: duration_api_ms.unwrap_or_default(),
        }))
    }

    pub fn boolean(&mut self, field: &str) -> Result<bool> {
        match self.next(field)? {
            Value::Integer(0) => Ok(false),
            Value::Integer(1) => Ok(true),
            other => bail!("{field} is not 0 or 1: {other:?}"),
        }
    }
}

/// `value` as an SQL integer, saturating at `i64::MAX`.
pub fn integer(value: u64) -> Value {
    Value::Integer(i64::try_from(value).unwrap_or(i64::MAX))
}

/// `value` as an SQL integer, or NULL.
pub fn optional_integer(value: Option<u64>) -> Value {
    value.map_or(Value::Null, integer)
}

/// `value` as SQL text, or NULL.
pub fn optional_text(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |text| Value::Text(text.to_owned()))
}

/// `value` as an SQL integer, 0 or 1.
pub fn boolean(value: bool) -> Value {
    Value::Integer(i64::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_past_i64_saturates() {
        assert_eq!(integer(u64::MAX), Value::Integer(i64::MAX));
        assert_eq!(integer(7), Value::Integer(7));
        assert_eq!(optional_integer(None), Value::Null);
        assert_eq!(boolean(true), Value::Integer(1));
    }
}

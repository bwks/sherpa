use std::fmt;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// Database-neutral record identifier used across Sherpa crates.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct RecordId {
    pub table: String,
    pub key: RecordIdKey,
}

/// Record key variants currently produced and consumed by Sherpa.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum RecordIdKey {
    Number(i64),
    String(String),
}

impl RecordId {
    #[instrument(level = "debug", skip_all)]
    pub fn new(table: impl Into<String>, key: impl Into<RecordIdKey>) -> Self {
        Self {
            table: table.into(),
            key: key.into(),
        }
    }

    /// Parse a record ID written as `table:key`.
    ///
    /// # Errors
    ///
    /// Fails when `value` is not a `table:key` record ID.
    #[instrument(level = "debug")]
    pub fn parse_simple(value: &str) -> Result<Self> {
        let (table, key) = value
            .split_once(':')
            .context("Record ID must use the 'table:key' format")?;
        if table.is_empty() {
            bail!("Record ID table cannot be empty");
        }
        if key.is_empty() {
            bail!("Record ID key cannot be empty");
        }
        Ok(Self::new(table, key))
    }
}

impl From<String> for RecordIdKey {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for RecordIdKey {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<i64> for RecordIdKey {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

impl fmt::Display for RecordIdKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::String(value) => write!(formatter, "{value}"),
        }
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.table, self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_id_parse_simple() {
        let id = RecordId::parse_simple("lab:abcd").unwrap();

        assert_eq!(id, RecordId::new("lab", "abcd"));
    }

    #[test]
    fn test_record_id_parse_rejects_invalid_values() {
        assert!(RecordId::parse_simple("missing-separator").is_err());
        assert!(RecordId::parse_simple(":missing-table").is_err());
        assert!(RecordId::parse_simple("node:").is_err());
    }
}

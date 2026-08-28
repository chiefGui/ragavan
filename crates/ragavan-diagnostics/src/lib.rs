#![forbid(unsafe_code)]

use std::error::Error;

/// A failure with stable, actionable metadata for people and tools.
pub trait Diagnostic: Error {
    /// Return the stable machine-readable identity of this failure.
    fn code(&self) -> &'static str;

    /// Return an action that may resolve the failure.
    fn help(&self) -> Option<String> {
        None
    }

    /// Return structured values that explain this specific failure.
    fn details(&self) -> Vec<Detail> {
        Vec::new()
    }
}

/// One named value attached to a diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Detail {
    name: &'static str,
    value: Value,
}

impl Detail {
    pub fn text(name: &'static str, value: impl Into<String>) -> Self {
        Self {
            name,
            value: Value::Text(value.into()),
        }
    }

    pub const fn number(name: &'static str, value: u64) -> Self {
        Self {
            name,
            value: Value::Number(value),
        }
    }

    pub fn list(name: &'static str, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            name,
            value: Value::List(values.into_iter().map(Into::into).collect()),
        }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn value(&self) -> &Value {
        &self.value
    }
}

/// A structured diagnostic value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Text(String),
    Number(u64),
    List(Vec<String>),
}

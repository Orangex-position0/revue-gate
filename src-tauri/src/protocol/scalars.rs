use serde_json::Value;

use crate::protocol::ProtocolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenLimit(u64);

impl TokenLimit {
    pub(crate) fn parse(value: &Value, field: &'static str) -> Result<Self, ProtocolError> {
        let n = value.as_u64().ok_or_else(|| {
            ProtocolError::invalid(field, "token limit must be a non-negative integer")
        })?;
        Ok(Self(n))
    }

    pub(crate) fn into_value(self) -> Value {
        Value::from(self.0)
    }
}

pub(crate) fn required_non_empty_string(
    body: &Value,
    field: &'static str,
) -> Result<String, ProtocolError> {
    match body.pointer(field) {
        None => Err(ProtocolError::missing(field)),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.clone()),
        Some(Value::String(_)) => Err(ProtocolError::missing(field)),
        Some(_) => Err(ProtocolError::invalid(field, "field must be a string")),
    }
}

pub(crate) fn optional_bool(body: &Value, field: &'static str) -> Result<bool, ProtocolError> {
    match body.pointer(field) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(ProtocolError::invalid(field, "stream must be a boolean")),
    }
}

pub(crate) fn optional_number_value(
    body: &Value,
    field: &'static str,
) -> Result<Option<Value>, ProtocolError> {
    match body.pointer(field) {
        None => Ok(None),
        Some(value) if value.is_number() => Ok(Some(value.clone())),
        Some(_) => Err(ProtocolError::invalid(field, "field must be a number")),
    }
}

pub(crate) fn object<'a>(
    value: &'a Value,
    field: &str,
) -> Result<&'a serde_json::Map<String, Value>, ProtocolError> {
    value
        .as_object()
        .ok_or_else(|| ProtocolError::invalid(field, "field must be an object"))
}

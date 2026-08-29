#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The incoming JSON is syntactically valid but does not match the expected top-level request shape.
    #[error("malformed request: {reason}")]
    MalformedRequest { reason: String },
    /// The request uses a protocol feature that cannot be represented safely in canonical Chat.
    #[error("unsupported feature at {field}: {reason}")]
    UnsupportedFeature { field: String, reason: String },
    /// A field required by the downstream protocol or canonical conversion is absent.
    #[error("missing required field: {field}")]
    MissingRequiredField { field: String },
    /// A known field is present but has the wrong type, value, or semantic shape.
    #[error("invalid field at {field}: {reason}")]
    InvalidField { field: String, reason: String },
    /// Streaming input cannot be parsed or converted at the current SSE boundary.
    #[error("stream error: {reason}")]
    Stream { reason: String },
}

impl ProtocolError {
    pub fn malformed(reason: impl Into<String>) -> Self {
        Self::MalformedRequest {
            reason: reason.into(),
        }
    }

    pub fn unsupported(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::UnsupportedFeature {
            field: field.into(),
            reason: reason.into(),
        }
    }

    pub fn missing(field: impl Into<String>) -> Self {
        Self::MissingRequiredField {
            field: field.into(),
        }
    }

    pub fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidField {
            field: field.into(),
            reason: reason.into(),
        }
    }

    pub fn stream(reason: impl Into<String>) -> Self {
        Self::Stream {
            reason: reason.into(),
        }
    }
}

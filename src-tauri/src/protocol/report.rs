use crate::protocol::ProtocolKind;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversionReport {
    pub source_protocol: ProtocolKind,
    pub target_protocol: ProtocolKind,
    pub mapped_fields: Vec<FieldMapping>,
    pub warnings: Vec<ConversionWarning>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldMapping {
    pub source: String,
    pub target: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ConversionWarningKind {
    StrippedAnnotation,
    NormalizedField,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversionWarning {
    pub kind: ConversionWarningKind,
    pub field: String,
    pub message: String,
}

impl ConversionReport {
    pub fn new(source_protocol: ProtocolKind, target_protocol: ProtocolKind) -> Self {
        Self {
            source_protocol,
            target_protocol,
            mapped_fields: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn map(&mut self, source: impl Into<String>, target: impl Into<String>) {
        self.mapped_fields.push(FieldMapping {
            source: source.into(),
            target: target.into(),
            note: None,
        });
    }

    pub fn map_with_note(
        &mut self,
        source: impl Into<String>,
        target: impl Into<String>,
        note: impl Into<String>,
    ) {
        self.mapped_fields.push(FieldMapping {
            source: source.into(),
            target: target.into(),
            note: Some(note.into()),
        });
    }

    pub fn warn(
        &mut self,
        kind: ConversionWarningKind,
        field: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.warnings.push(ConversionWarning {
            kind,
            field: field.into(),
            message: message.into(),
        });
    }
}

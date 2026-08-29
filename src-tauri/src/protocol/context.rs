#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversionContext {
    pub trace_id: String,
    pub now_unix: i64,
}

use bytes::Bytes;
use serde_json::Value;

use crate::protocol::scalars::{optional_bool, required_non_empty_string};
use crate::protocol::{
    ConversionContext, ConversionReport, ConvertedRequest, ConvertedResponse, ProtocolError,
    ProtocolKind, StreamFrame, StreamTransformer,
};

pub fn passthrough_request(
    request: Value,
    _context: &ConversionContext,
) -> Result<ConvertedRequest, ProtocolError> {
    if !request.is_object() {
        return Err(ProtocolError::malformed(
            "request body must be a JSON object",
        ));
    }
    required_non_empty_string(&request, "/model")?;
    match request.pointer("/messages") {
        None => return Err(ProtocolError::missing("/messages")),
        Some(Value::Array(_)) => {}
        Some(_) => {
            return Err(ProtocolError::invalid(
                "/messages",
                "messages must be an array",
            ));
        }
    }
    let stream = optional_bool(&request, "/stream")?;
    Ok(ConvertedRequest {
        body: request,
        stream,
        report: ConversionReport::new(ProtocolKind::OpenAiChat, ProtocolKind::OpenAiChat),
    })
}

pub fn passthrough_response(
    response: Value,
    _context: &ConversionContext,
) -> Result<ConvertedResponse, ProtocolError> {
    Ok(ConvertedResponse {
        body: response,
        report: ConversionReport::new(ProtocolKind::OpenAiChat, ProtocolKind::OpenAiChat),
    })
}

pub mod stream {
    use super::*;

    pub struct PassthroughStreamTransformer;

    impl StreamTransformer for PassthroughStreamTransformer {
        fn transform_chunk(&mut self, chunk: Bytes) -> Result<Vec<StreamFrame>, ProtocolError> {
            Ok(vec![StreamFrame { bytes: chunk }])
        }

        fn finish(&mut self) -> Result<Vec<StreamFrame>, ProtocolError> {
            Ok(Vec::new())
        }
    }
}

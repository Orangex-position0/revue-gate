pub mod response;
pub mod stream;

use serde_json::Value;

use crate::protocol::{ConversionContext, ConvertedRequest, ConvertedResponse, ProtocolError};

pub fn to_openai_chat(
    request: Value,
    context: &ConversionContext,
) -> Result<ConvertedRequest, ProtocolError> {
    response::to_openai_chat(request, context)
}

pub fn from_openai_chat(
    response: Value,
    context: &ConversionContext,
) -> Result<ConvertedResponse, ProtocolError> {
    response::from_openai_chat(response, context)
}

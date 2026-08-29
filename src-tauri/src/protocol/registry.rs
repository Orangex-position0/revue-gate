use bytes::Bytes;
use serde_json::Value;

use crate::protocol::{
    ConversionContext, ConversionReport, ProtocolError, anthropic, openai_chat, responses,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProtocolKind {
    OpenAiChat,
    AnthropicMessages,
    OpenAiResponses,
}

impl ProtocolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProtocolKind::OpenAiChat => "openai_chat",
            ProtocolKind::AnthropicMessages => "anthropic_messages",
            ProtocolKind::OpenAiResponses => "openai_responses",
        }
    }
}

impl std::fmt::Display for ProtocolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConvertedRequest {
    pub body: Value,
    pub stream: bool,
    pub report: ConversionReport,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConvertedResponse {
    pub body: Value,
    pub report: ConversionReport,
}

#[derive(Debug, Clone)]
pub struct StreamFrame {
    pub bytes: Bytes,
}

pub trait StreamTransformer {
    fn transform_chunk(&mut self, chunk: Bytes) -> Result<Vec<StreamFrame>, ProtocolError>;
    fn finish(&mut self) -> Result<Vec<StreamFrame>, ProtocolError>;
}

pub struct CodecRegistry;

impl CodecRegistry {
    pub fn convert_request(
        protocol: ProtocolKind,
        request: Value,
        context: &ConversionContext,
    ) -> Result<ConvertedRequest, ProtocolError> {
        match protocol {
            ProtocolKind::OpenAiChat => openai_chat::passthrough_request(request, context),
            ProtocolKind::AnthropicMessages => anthropic::to_openai_chat(request, context),
            ProtocolKind::OpenAiResponses => responses::to_openai_chat(request, context),
        }
    }

    pub fn convert_response(
        protocol: ProtocolKind,
        response: Value,
        context: &ConversionContext,
    ) -> Result<ConvertedResponse, ProtocolError> {
        match protocol {
            ProtocolKind::OpenAiChat => openai_chat::passthrough_response(response, context),
            ProtocolKind::AnthropicMessages => anthropic::from_openai_chat(response, context),
            ProtocolKind::OpenAiResponses => responses::from_openai_chat(response, context),
        }
    }

    pub fn stream_transformer(
        protocol: ProtocolKind,
        context: &ConversionContext,
    ) -> Box<dyn StreamTransformer + Send> {
        match protocol {
            ProtocolKind::OpenAiChat => Box::new(openai_chat::stream::PassthroughStreamTransformer),
            ProtocolKind::AnthropicMessages => {
                Box::new(anthropic::stream::AnthropicStreamTransformer::new(context))
            }
            ProtocolKind::OpenAiResponses => {
                Box::new(responses::stream::ResponsesStreamTransformer::new(context))
            }
        }
    }
}

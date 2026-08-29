use bytes::{Bytes, BytesMut};

use crate::protocol::ProtocolError;

pub(crate) struct SseDecoder {
    buffer: BytesMut,
}

pub(crate) struct SseRecord {
    pub event: Option<String>,
    pub data: String,
}

impl SseDecoder {
    pub(crate) fn new() -> Self {
        Self {
            buffer: BytesMut::new(),
        }
    }

    pub(crate) fn push(&mut self, chunk: Bytes) -> Result<Vec<SseRecord>, ProtocolError> {
        self.buffer.extend_from_slice(&chunk);
        let mut records = Vec::new();
        while let Some((end, sep_len)) = find_record_end(&self.buffer) {
            let bytes = self.buffer.split_to(end);
            self.buffer.advance(sep_len);
            if let Some(record) = parse_record(&bytes)? {
                records.push(record);
            }
        }
        Ok(records)
    }

    pub(crate) fn finish(&mut self) -> Result<(), ProtocolError> {
        if self.buffer.iter().all(u8::is_ascii_whitespace) {
            self.buffer.clear();
            Ok(())
        } else {
            Err(ProtocolError::stream("incomplete SSE record"))
        }
    }
}

fn find_record_end(buffer: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buffer.len().saturating_sub(1) {
        if buffer[i] == b'\n' && buffer[i + 1] == b'\n' {
            return Some((i, 2));
        }
        if i + 3 < buffer.len()
            && buffer[i] == b'\r'
            && buffer[i + 1] == b'\n'
            && buffer[i + 2] == b'\r'
            && buffer[i + 3] == b'\n'
        {
            return Some((i, 4));
        }
    }
    None
}

fn parse_record(bytes: &[u8]) -> Result<Option<SseRecord>, ProtocolError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ProtocolError::stream(e.to_string()))?;
    let mut event = None;
    let mut data_lines = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }
    if data_lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(SseRecord {
        event,
        data: data_lines.join("\n"),
    }))
}

use bytes::Buf;

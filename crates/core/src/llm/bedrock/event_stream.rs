//! Incremental AWS EventStream framing (prelude/message CRCs included).
use crate::error::CoreError;
use bytes::{Buf, BytesMut};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Decoder {
    buffer: BytesMut,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<(String, serde_json::Value)>, CoreError> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while self.buffer.len() >= 12 {
            let total = u32::from_be_bytes(self.buffer[..4].try_into().unwrap()) as usize;
            let headers = u32::from_be_bytes(self.buffer[4..8].try_into().unwrap()) as usize;
            let prelude_crc = u32::from_be_bytes(self.buffer[8..12].try_into().unwrap());
            if !(16..=MAX_FRAME_BYTES).contains(&total)
                || headers > total - 16
                || crc32fast::hash(&self.buffer[..8]) != prelude_crc
            {
                return Err(invalid("invalid prelude or frame size"));
            }
            if self.buffer.len() < total {
                break;
            }
            let crc = u32::from_be_bytes(self.buffer[total - 4..total].try_into().unwrap());
            if crc32fast::hash(&self.buffer[..total - 4]) != crc {
                return Err(invalid("message CRC mismatch"));
            }
            let mut offset = 12;
            let end = 12 + headers;
            let mut event_type = None;
            let mut message_type = None;
            while offset < end {
                let len = self.buffer[offset] as usize;
                offset += 1;
                if offset + len + 1 > end {
                    return Err(invalid("truncated header"));
                }
                let name = std::str::from_utf8(&self.buffer[offset..offset + len])
                    .map_err(|_| invalid("invalid header name"))?;
                offset += len;
                let kind = self.buffer[offset];
                offset += 1;
                let len = match kind {
                    0 | 1 => 0,
                    2 => 1,
                    3 => 2,
                    4 => 4,
                    5 | 8 => 8,
                    9 => 16,
                    6 | 7 => {
                        if offset + 2 > end {
                            return Err(invalid("truncated header size"));
                        }
                        let len =
                            u16::from_be_bytes(self.buffer[offset..offset + 2].try_into().unwrap())
                                as usize;
                        offset += 2;
                        len
                    }
                    _ => return Err(invalid("unknown header type")),
                };
                if offset + len > end {
                    return Err(invalid("truncated header value"));
                }
                if matches!(name, ":event-type" | ":exception-type" | ":message-type") {
                    if kind != 7 {
                        return Err(invalid("non-string event header"));
                    }
                    let value = std::str::from_utf8(&self.buffer[offset..offset + len])
                        .map_err(|_| invalid("invalid event header"))?
                        .to_owned();
                    if name == ":message-type" {
                        message_type = Some(value);
                    } else {
                        event_type = Some(value);
                    }
                }
                offset += len;
            }
            let event_type = event_type.ok_or_else(|| invalid("missing event type"))?;
            if !matches!(
                message_type.as_deref(),
                Some("event" | "exception" | "error")
            ) {
                return Err(invalid("missing message type"));
            }
            let value = serde_json::from_slice(&self.buffer[end..total - 4])
                .map_err(|_| invalid("invalid event JSON"))?;
            events.push((event_type, value));
            self.buffer.advance(total);
        }
        Ok(events)
    }

    pub fn finish(&self) -> Result<(), CoreError> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(invalid("truncated final frame"))
        }
    }
}

fn invalid(reason: &str) -> CoreError {
    CoreError::StreamIncomplete(format!("Bedrock EventStream: {reason}"))
}

#[cfg(test)]
pub(super) fn frame(kind: &str, value: serde_json::Value) -> Vec<u8> {
    let mut headers = Vec::new();
    for (name, value) in [(":message-type", "event"), (":event-type", kind)] {
        headers.push(name.len() as u8);
        headers.extend_from_slice(name.as_bytes());
        headers.push(7);
        headers.extend_from_slice(&(value.len() as u16).to_be_bytes());
        headers.extend_from_slice(value.as_bytes());
    }
    let body = serde_json::to_vec(&value).unwrap();
    let total = 16 + headers.len() + body.len();
    let mut result = Vec::new();
    result.extend_from_slice(&(total as u32).to_be_bytes());
    result.extend_from_slice(&(headers.len() as u32).to_be_bytes());
    result.extend_from_slice(&crc32fast::hash(&result).to_be_bytes());
    result.extend(headers);
    result.extend(body);
    result.extend_from_slice(&crc32fast::hash(&result).to_be_bytes());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_frames_validate_both_checksums_and_eof() {
        let input = frame("messageStart", serde_json::json!({"role":"assistant"}));
        for size in 1..input.len() {
            let mut decoder = Decoder::default();
            let events: Vec<_> = input
                .chunks(size)
                .flat_map(|chunk| decoder.push(chunk).unwrap())
                .collect();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].0, "messageStart");
            decoder.finish().unwrap();
        }
        for index in [0, 8, input.len() - 5] {
            let mut broken = input.clone();
            broken[index] ^= 1;
            assert!(Decoder::default().push(&broken).is_err());
        }
        let mut partial = Decoder::default();
        partial.push(&input[..input.len() - 1]).unwrap();
        assert!(partial.finish().is_err());
    }
}

//! Bounded text previews with the hash of the same byte stream.
use std::fs::File;
use std::io::{self, BufRead, BufReader};

pub struct TextWindow {
    pub bytes: Vec<u8>,
    pub hash: String,
    pub truncated: bool,
    pub binary: bool,
}

/// Skip to an original one-based line, retaining at most one preview window.
/// Byte offsets are aligned to UTF-16 code units when the source has a BOM.
pub fn read(file: File, first_line: u32, max_bytes: usize) -> io::Result<TextWindow> {
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let prefix = reader.fill_buf()?;
    let binary = crate::parse::bytes_appear_binary(prefix);
    let bom = match prefix {
        [0xff, 0xfe, ..] => Some([0xff, 0xfe]),
        [0xfe, 0xff, ..] => Some([0xfe, 0xff]),
        _ => None,
    };
    let mut skip = first_line.saturating_sub(1);
    let mut bytes = Vec::new();
    // Keep the encoding declaration when the excerpt starts after the BOM.
    if skip > 0 {
        if let Some(bom) = bom {
            bytes.extend_from_slice(&bom);
        }
    }
    let mut hasher = blake3::Hasher::new();
    let mut offset = 0u64;
    let mut previous = 0u8;
    let mut truncated = skip > 0;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            break;
        }
        let mut start = 0;
        if skip > 0 {
            start = buf.len();
            for (i, &byte) in buf.iter().enumerate() {
                let newline = match bom {
                    Some([0xff, _]) => (offset + i as u64) % 2 == 1 && previous == 10 && byte == 0,
                    Some(_) => (offset + i as u64) % 2 == 1 && previous == 0 && byte == 10,
                    None => byte == 10,
                };
                previous = byte;
                if newline {
                    skip -= 1;
                    if skip == 0 {
                        start = i + 1;
                        break;
                    }
                }
            }
        }
        let available = &buf[start..];
        let take = available.len().min(max_bytes.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&available[..take]);
        let full = take < available.len();
        let len = buf.len();
        hasher.update(buf);
        offset += len as u64;
        reader.consume(len);
        if full {
            truncated = true;
            hasher.update_reader(reader)?;
            // A byte-limited UTF-8 preview can end inside a character.
            if bom.is_none() {
                if let Err(error) = std::str::from_utf8(&bytes) {
                    if error.error_len().is_none() {
                        bytes.truncate(error.valid_up_to());
                    }
                }
            }
            break;
        }
    }
    Ok(TextWindow {
        bytes,
        hash: hasher.finalize().to_hex().to_string(),
        truncated,
        binary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};
    #[test]
    fn reads_late_utf16_lines_without_confusing_code_units_with_newlines() {
        let mut bytes = vec![0xff, 0xfe];
        for unit in "ਊ"
            .repeat(40_000)
            .encode_utf16()
            .chain("\r\nthird\nend".encode_utf16())
        {
            bytes.extend(unit.to_le_bytes());
        }
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        file.rewind().unwrap();
        let window = read(file, 2, 64).unwrap();
        assert_eq!(window.hash, blake3::hash(&bytes).to_hex().as_str());
        assert_eq!(
            crate::parse::decode_text_bytes(&window.bytes, std::path::Path::new("test")).unwrap(),
            "third\nend"
        );
    }
}

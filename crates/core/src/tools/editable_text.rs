//! Lossless decoding for mutations. Search/index readers may guess encodings;
//! editing must preserve a known encoding or fail before touching the file.
use std::path::Path;

use super::document_utils::edit_guidance_for_path;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

pub(super) fn validate_utf8_append(path: &Path) -> Result<(), String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buffer = [0_u8; 64 * 1024 + 4];
    let mut pending = 0;
    loop {
        let count = file
            .read(&mut buffer[pending..])
            .map_err(|e| e.to_string())?;
        let bytes = &buffer[..pending + count];
        if bytes.contains(&0) {
            return Err("Append requires a UTF-8 text file; use an encoding-preserving editor for this file".into());
        }
        match std::str::from_utf8(bytes) {
            Ok(_) => pending = 0,
            Err(error) if error.error_len().is_none() && count > 0 => {
                let start = error.valid_up_to();
                pending = bytes.len() - start;
                buffer.copy_within(start..start + pending, 0);
            }
            Err(_) => return Err("Append requires a UTF-8 text file; use edit_file or multi_edit for BOM-marked UTF-16, or convert with an explicit encoding first".into()),
        }
        if count == 0 {
            return Ok(());
        }
    }
}

enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

pub(super) struct EditableText {
    pub text: String,
    encoding: Encoding,
}

impl EditableText {
    pub fn read(path: &Path) -> Result<Self, String> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|e| format!("Cannot read file: {e}"))?;
        let mut raw = Vec::new();
        file.take(MAX_FILE_SIZE + 1)
            .read_to_end(&mut raw)
            .map_err(|e| e.to_string())?;
        if raw.len() as u64 > MAX_FILE_SIZE {
            return Err(format!(
                "File too large (limit is {} MB): {}",
                MAX_FILE_SIZE / (1024 * 1024),
                path.display()
            ));
        }
        let (encoding, body) = if raw.starts_with(b"\xef\xbb\xbf") {
            (Encoding::Utf8Bom, &raw[3..])
        } else if raw.starts_with(b"\xff\xfe") {
            (Encoding::Utf16Le, &raw[2..])
        } else if raw.starts_with(b"\xfe\xff") {
            (Encoding::Utf16Be, &raw[2..])
        } else {
            (Encoding::Utf8, raw.as_slice())
        };
        let text = match encoding {
            Encoding::Utf8 | Encoding::Utf8Bom => {
                if body.contains(&0) {
                    return Err(edit_guidance_for_path(path).unwrap_or_else(|| {
                        format!("File appears to be binary: {}", path.display())
                    }));
                }
                std::str::from_utf8(body).map(str::to_owned).map_err(|_| format!(
                    "Cannot edit '{}' losslessly: encoding is not UTF-8 or BOM-marked UTF-16. Convert it with an explicitly selected encoding first.", path.display()))?
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                if body.len() % 2 != 0 {
                    return Err(
                        "Invalid UTF-16: incomplete code unit; file was not modified".into(),
                    );
                }
                let units = body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| match encoding {
                        Encoding::Utf16Le => u16::from_le_bytes([pair[0], pair[1]]),
                        _ => u16::from_be_bytes([pair[0], pair[1]]),
                    })
                    .collect::<Vec<_>>();
                String::from_utf16(&units)
                    .map_err(|_| "Invalid UTF-16: unpaired surrogate; file was not modified")?
            }
        };
        Ok(Self { text, encoding })
    }

    pub fn encode(&self, text: &str) -> Vec<u8> {
        match self.encoding {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Utf8Bom => [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
            Encoding::Utf16Le => [
                vec![0xff, 0xfe],
                text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ]
            .concat(),
            Encoding::Utf16Be => [
                vec![0xfe, 0xff],
                text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            ]
            .concat(),
        }
    }
}

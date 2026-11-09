//! Request body decoding: decompression with a size cap, then splitting into
//! items (a JSON array or NDJSON).

use std::io::Read;

use serde_json::value::RawValue;

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    UnsupportedEncoding(String),
    TooLarge,
    Corrupt(String),
    BadJson(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedEncoding(e) => write!(f, "unsupported Content-Encoding: {e}"),
            Self::TooLarge => write!(f, "decompressed body exceeds the limit"),
            Self::Corrupt(e) => write!(f, "could not decompress body: {e}"),
            Self::BadJson(e) => write!(f, "invalid JSON: {e}"),
        }
    }
}

/// Decompresses `body` according to `Content-Encoding`, refusing to inflate
/// beyond `max` bytes (zip-bomb guard).
pub fn decompress(encoding: Option<&str>, body: &[u8], max: usize) -> Result<Vec<u8>, DecodeError> {
    let enc = encoding.map(|e| e.trim().to_ascii_lowercase());
    match enc.as_deref() {
        None | Some("") | Some("identity") => {
            if body.len() > max {
                Err(DecodeError::TooLarge)
            } else {
                Ok(body.to_vec())
            }
        }
        Some("gzip") | Some("x-gzip") => read_capped(flate2::read::MultiGzDecoder::new(body), max),
        Some("zstd") => {
            let dec = zstd::stream::read::Decoder::new(body)
                .map_err(|e| DecodeError::Corrupt(e.to_string()))?;
            read_capped(dec, max)
        }
        Some(other) => Err(DecodeError::UnsupportedEncoding(other.to_string())),
    }
}

fn read_capped(reader: impl Read, max: usize) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::new();
    reader
        .take(max as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| DecodeError::Corrupt(e.to_string()))?;
    if out.len() > max {
        return Err(DecodeError::TooLarge);
    }
    Ok(out)
}

/// Splits the body into raw item JSON: either one JSON array, or NDJSON (one
/// item per non-empty line). Items are parsed individually afterwards so one
/// bad item doesn't reject the batch.
pub fn split_items(buf: &[u8]) -> Result<Vec<&str>, DecodeError> {
    let text = std::str::from_utf8(buf).map_err(|e| DecodeError::BadJson(e.to_string()))?;
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        let items: Vec<&RawValue> =
            serde_json::from_str(trimmed).map_err(|e| DecodeError::BadJson(e.to_string()))?;
        return Ok(items.into_iter().map(RawValue::get).collect());
    }
    Ok(trimmed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn decompresses_gzip_and_zstd() {
        let data = br#"[{"type":"event"}]"#;
        assert_eq!(decompress(Some("gzip"), &gzip(data), 1024).unwrap(), data);
        let z = zstd::encode_all(&data[..], 3).unwrap();
        assert_eq!(decompress(Some("zstd"), &z, 1024).unwrap(), data);
        assert_eq!(decompress(None, data, 1024).unwrap(), data);
    }

    #[test]
    fn caps_decompressed_size() {
        let bomb = gzip(&vec![b'a'; 100_000]);
        assert!(bomb.len() < 1_000);
        assert_eq!(
            decompress(Some("gzip"), &bomb, 10_000),
            Err(DecodeError::TooLarge)
        );
    }

    #[test]
    fn rejects_unknown_encoding_and_garbage() {
        assert!(matches!(
            decompress(Some("br"), b"x", 10),
            Err(DecodeError::UnsupportedEncoding(_))
        ));
        assert!(matches!(
            decompress(Some("gzip"), b"not gzip", 10),
            Err(DecodeError::Corrupt(_))
        ));
    }

    #[test]
    fn splits_arrays_and_ndjson() {
        assert_eq!(
            split_items(br#"[{"a":1}, {"b":[2]}]"#).unwrap(),
            vec![r#"{"a":1}"#, r#"{"b":[2]}"#]
        );
        assert_eq!(split_items(b"{\"a\":1}\n\n{\"b\":2}\r\n").unwrap().len(), 2);
        assert!(split_items(b"").unwrap().is_empty());
        assert!(split_items(b"[{").is_err());
    }
}

//! Detection and conversion of text encodings (05. File formats, section 1.1).

use crate::error::{CoreError, Result};
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};

/// Text decoded to UTF-8, with the encoding information that persist needs.
pub struct Decoded {
    pub text: String,
    pub encoding: &'static Encoding,
    pub bom: bool,
}

/// Finds an encoding by a name or a label (`cp932` is also accepted).
pub fn lookup(name: &str) -> Result<&'static Encoding> {
    let label = match name.trim().to_ascii_lowercase().as_str() {
        "cp932" | "ms932" | "sjis" | "shift-jis" => "shift_jis",
        "eucjp" => "euc-jp",
        "utf8" => "utf-8",
        "utf-8-bom" | "utf8bom" => "utf-8",
        "iso-8859-1" | "latin1" => "windows-1252",
        _ => name.trim(),
    }
    .to_string();
    Encoding::for_label(label.as_bytes())
        .ok_or_else(|| CoreError::InvalidArgument(format!("unknown encoding: {name}")))
}

/// Detects the encoding of the bytes (BOM → valid UTF-8 → chardetng) and decodes them.
///
/// If `forced` is given, Armarium uses it. A BOM is still removed.
pub fn decode(bytes: &[u8], forced: Option<&str>, path_for_errors: &str) -> Result<Decoded> {
    let bom = Encoding::for_bom(bytes);
    if let Some(name) = forced {
        let enc = lookup(name)?;
        let (bom_present, skip) = match bom {
            Some((b, n)) if b == enc => (true, n),
            _ => (false, 0),
        };
        let (text, _had_errors) = enc.decode_without_bom_handling(&bytes[skip..]);
        return Ok(Decoded {
            text: text.into_owned(),
            encoding: enc,
            bom: bom_present,
        });
    }
    if let Some((enc, n)) = bom {
        let (text, had_errors) = enc.decode_without_bom_handling(&bytes[n..]);
        if had_errors {
            return Err(CoreError::EncodingUndetected(path_for_errors.to_string()));
        }
        return Ok(Decoded {
            text: text.into_owned(),
            encoding: enc,
            bom: true,
        });
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(Decoded {
            text: s.to_string(),
            encoding: UTF_8,
            bom: false,
        });
    }
    let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    let sample = &bytes[..bytes.len().min(1 << 20)];
    det.feed(sample, sample.len() == bytes.len());
    let enc = det.guess(Some(b"jp"), chardetng::Utf8Detection::Allow);
    let (text, had_errors) = enc.decode_without_bom_handling(bytes);
    if had_errors {
        return Err(CoreError::EncodingUndetected(path_for_errors.to_string()));
    }
    Ok(Decoded {
        text: text.into_owned(),
        encoding: enc,
        bom: false,
    })
}

/// Encodes UTF-8 text in the encoding of a file.
pub struct TextEncoder {
    kind: Kind,
}

enum Kind {
    Utf8,
    Utf16 { little_endian: bool },
    Legacy(encoding_rs::Encoder),
}

impl TextEncoder {
    pub fn new(encoding: &'static Encoding) -> TextEncoder {
        let kind = if encoding == UTF_8 {
            Kind::Utf8
        } else if encoding == UTF_16LE {
            Kind::Utf16 { little_endian: true }
        } else if encoding == UTF_16BE {
            Kind::Utf16 { little_endian: false }
        } else {
            Kind::Legacy(encoding.new_encoder())
        };
        TextEncoder { kind }
    }

    /// The BOM bytes of the encoding.
    pub fn bom(&self) -> &'static [u8] {
        match self.kind {
            Kind::Utf8 => b"\xEF\xBB\xBF",
            Kind::Utf16 { little_endian: true } => b"\xFF\xFE",
            Kind::Utf16 { little_endian: false } => b"\xFE\xFF",
            Kind::Legacy(_) => b"",
        }
    }

    /// Appends the encoded text to `out`. Returns the first character that the
    /// encoding cannot represent; in this case `out` is incomplete.
    pub fn encode(&mut self, text: &str, out: &mut Vec<u8>) -> std::result::Result<(), char> {
        match &mut self.kind {
            Kind::Utf8 => {
                out.extend_from_slice(text.as_bytes());
                Ok(())
            }
            Kind::Utf16 { little_endian } => {
                for u in text.encode_utf16() {
                    let b = if *little_endian { u.to_le_bytes() } else { u.to_be_bytes() };
                    out.extend_from_slice(&b);
                }
                Ok(())
            }
            Kind::Legacy(enc) => {
                let mut src = text;
                loop {
                    let need = enc
                        .max_buffer_length_from_utf8_without_replacement(src.len())
                        .unwrap_or(src.len() * 4 + 16);
                    out.reserve(need);
                    let (res, read) = enc.encode_from_utf8_to_vec_without_replacement(src, out, false);
                    src = &src[read..];
                    match res {
                        encoding_rs::EncoderResult::InputEmpty => return Ok(()),
                        encoding_rs::EncoderResult::OutputFull => continue,
                        encoding_rs::EncoderResult::Unmappable(c) => return Err(c),
                    }
                }
            }
        }
    }

    /// True if the encoding can represent the text.
    pub fn can_encode(&mut self, text: &str) -> bool {
        match self.kind {
            Kind::Utf8 | Kind::Utf16 { .. } => true,
            Kind::Legacy(_) => {
                if text.is_ascii() {
                    return true;
                }
                let mut tmp = Vec::new();
                self.encode(text, &mut tmp).is_ok()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_encodings() {
        let d = decode("a,b\n1,2\n".as_bytes(), None, "x").unwrap();
        assert_eq!(d.encoding, UTF_8);
        assert!(!d.bom);

        let d = decode(b"\xEF\xBB\xBFa,b\n", None, "x").unwrap();
        assert!(d.bom);
        assert_eq!(d.text, "a,b\n");

        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("部署,金額\n営業,1200000\n開発,800000\n");
        let d = decode(&sjis, None, "x").unwrap();
        assert_eq!(d.encoding, encoding_rs::SHIFT_JIS);
        assert!(d.text.starts_with("部署"));

        let mut u16: Vec<u8> = vec![0xFF, 0xFE];
        for u in "a,b\n".encode_utf16() {
            u16.extend_from_slice(&u.to_le_bytes());
        }
        let d = decode(&u16, None, "x").unwrap();
        assert_eq!(d.encoding, UTF_16LE);
        assert_eq!(d.text, "a,b\n");
    }

    #[test]
    fn encodes_and_reports_unmappable() {
        let mut e = TextEncoder::new(encoding_rs::SHIFT_JIS);
        let mut out = Vec::new();
        e.encode("営業", &mut out).unwrap();
        assert_eq!(encoding_rs::SHIFT_JIS.decode(&out).0, "営業");
        assert_eq!(e.encode("😀", &mut Vec::new()), Err('😀'));
        assert!(!e.can_encode("a😀"));
        assert_eq!(lookup("cp932").unwrap(), encoding_rs::SHIFT_JIS);
    }
}

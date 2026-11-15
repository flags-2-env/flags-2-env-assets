//! Dependency-free structural PNG integrity gate for versioned branding assets.
use std::fs;
use std::path::Path;

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = !0;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Validates chunk framing, CRCs, a bounded IHDR, IDAT presence and IEND.
/// This is a structural guard, not image decompression or metadata sanitizing.
fn validate_png(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() > MAX_PNG_BYTES {
        return Err("PNG exceeds size budget");
    }
    if !bytes.starts_with(SIGNATURE) {
        return Err("bad PNG signature");
    }
    let mut pos = SIGNATURE.len();
    let mut first = true;
    let mut idat_seen = false;
    while pos < bytes.len() {
        let header_end = pos.checked_add(12).ok_or("chunk header overflow")?;
        if header_end > bytes.len() {
            return Err("truncated PNG chunk header");
        }
        let length = u32::from_be_bytes(
            bytes[pos..pos + 4].try_into().map_err(|_| "invalid PNG length")?,
        ) as usize;
        let end = header_end.checked_add(length).ok_or("chunk length overflow")?;
        if end > bytes.len() {
            return Err("truncated PNG chunk");
        }
        let kind = &bytes[pos + 4..pos + 8];
        if !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err("invalid PNG chunk type");
        }
        let data = &bytes[pos + 8..pos + 8 + length];
        let stored = u32::from_be_bytes(
            bytes[pos + 8 + length..end]
                .try_into()
                .map_err(|_| "invalid PNG CRC")?,
        );
        if crc32(&bytes[pos + 4..pos + 8 + length]) != stored {
            return Err("PNG CRC mismatch");
        }
        if first {
            if kind != b"IHDR" || length != 13 {
                return Err("PNG must begin with a 13-byte IHDR");
            }
            let width = u32::from_be_bytes(data[0..4].try_into().map_err(|_| "bad width")?);
            let height = u32::from_be_bytes(data[4..8].try_into().map_err(|_| "bad height")?);
            if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
                return Err("invalid PNG dimensions");
            }
            if data[10] != 0 || data[11] != 0 || data[12] > 1 {
                return Err("unsupported PNG compression/filter/interlace");
            }
            first = false;
        } else if kind == b"IHDR" {
            return Err("duplicate PNG IHDR");
        }
        if kind == b"IDAT" {
            idat_seen = true;
        }
        pos = end;
        if kind == b"IEND" {
            if length != 0 || !idat_seen {
                return Err("invalid PNG IEND");
            }
            if pos != bytes.len() {
                return Err("trailing PNG data");
            }
            return Ok(());
        }
    }
    Err("missing PNG IEND")
}

fn visit(path: &Path) -> Result<usize, String> {
    let mut count = 0;
    for entry in fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_symlink() {
            return Err(format!("symlink in asset tree: {}", entry.path().display()));
        }
        if file_type.is_dir() {
            count += visit(&entry.path())?;
        } else if file_type.is_file()
            && entry.path().extension().is_some_and(|ext| ext == "png")
        {
            let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
            validate_png(&bytes).map_err(|e| format!("{}: {e}", entry.path().display()))?;
            count += 1;
        }
    }
    Ok(count)
}

fn main() {
    match visit(Path::new("branding")) {
        Ok(count) if count > 0 => println!("validated {count} PNG asset(s)"),
        Ok(_) => {
            eprintln!("no PNG assets discovered");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("asset validation failed: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(payload);
        let crc = crc32(&bytes[4..]);
        bytes.extend_from_slice(&crc.to_be_bytes());
        bytes
    }

    fn sample(width: u32) -> Vec<u8> {
        let mut png = SIGNATURE.to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&1_u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend(chunk(b"IHDR", &ihdr));
        png.extend(chunk(b"IDAT", &[1, 2, 3]));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn accepts_structurally_valid_bounded_png() {
        assert!(validate_png(&sample(1)).is_ok());
        assert!(validate_png(&sample(MAX_DIMENSION)).is_ok());
    }

    #[test]
    fn rejects_truncated_and_corrupt_png() {
        let mut png = sample(2);
        assert_eq!(validate_png(&png[..8]), Err("missing PNG IEND"));
        png[32] ^= 0xff;
        assert_eq!(validate_png(&png), Err("PNG CRC mismatch"));
        assert_eq!(validate_png(&sample(0)), Err("invalid PNG dimensions"));
        assert_eq!(validate_png(&sample(MAX_DIMENSION + 1)), Err("invalid PNG dimensions"));
    }

    #[test]
    fn rejects_missing_idat_and_trailing_garbage() {
        let mut without_idat = SIGNATURE.to_vec();
        let valid = sample(1);
        without_idat.extend_from_slice(&valid[8..33]);
        without_idat.extend(chunk(b"IEND", &[]));
        assert_eq!(validate_png(&without_idat), Err("invalid PNG IEND"));
        let mut extra = sample(1);
        extra.push(0);
        assert_eq!(validate_png(&extra), Err("trailing PNG data"));
    }

    #[test]
    fn enforces_maximum_blob_size() {
        let oversized = vec![0_u8; MAX_PNG_BYTES + 1];
        assert_eq!(validate_png(&oversized), Err("PNG exceeds size budget"));
    }
}

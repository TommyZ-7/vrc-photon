//! Image rendering for read-only external clients such as the MCP server.
//!
//! The daemon owns the database, so the MCP process only receives a display
//! path. Preview rendering and original-byte passthrough therefore happen in
//! this crate, close to the decoders used by the rest of the application.

use std::path::Path;

use crate::error::{CoreError, Result};

/// Longest edge of the JPEG preview returned to MCP clients.
pub const MCP_PREVIEW_MAX_EDGE: u32 = 1024;
/// JPEG quality used only for the MCP preview; originals are never re-encoded.
const MCP_PREVIEW_JPEG_QUALITY: u8 = 85;

/// Image bytes plus the MIME type MCP should advertise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoImage {
    pub bytes: Vec<u8>,
    pub mime_type: &'static str,
}

/// Renders a photo for an MCP image response.
///
/// `original = false` decodes `path`, fits it inside a 1024 px square, and
/// encodes a JPEG. `original = true` reads the file unchanged and only
/// determines its MIME type.
pub fn render_photo_image(path: &Path, original: bool) -> Result<PhotoImage> {
    if original {
        let bytes = std::fs::read(path)?;
        let mime_type = mime_type_for(path, &bytes).ok_or_else(|| {
            CoreError::Invalid(format!("unsupported image format: {}", path.display()))
        })?;
        return Ok(PhotoImage { bytes, mime_type });
    }

    let image = crate::image_decode::decode_image(path)
        .ok_or_else(|| CoreError::Invalid(format!("cannot decode image: {}", path.display())))?;
    let image = image.thumbnail(MCP_PREVIEW_MAX_EDGE, MCP_PREVIEW_MAX_EDGE);
    let bytes = crate::compressor::encode_jpeg(&image, MCP_PREVIEW_JPEG_QUALITY)?;
    Ok(PhotoImage {
        bytes,
        mime_type: "image/jpeg",
    })
}

/// Resolves the MIME type from the extension first and the file signature
/// second, so a misnamed or extensionless file still gets a usable type.
fn mime_type_for(path: &Path, bytes: &[u8]) -> Option<&'static str> {
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str())
        && let Some(mime_type) = mime_type_from_extension(extension)
    {
        return Some(mime_type);
    }
    mime_type_from_magic(bytes)
}

fn mime_type_from_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "avif" => Some("image/avif"),
        "jxl" => Some("image/jxl"),
        "tif" | "tiff" => Some("image/tiff"),
        _ => None,
    }
}

fn mime_type_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.get(0..4) == Some(&b"RIFF"[..]) && bytes.get(8..12) == Some(&b"WEBP"[..]) {
        return Some("image/webp");
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return Some("image/tiff");
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"avif" || brand == b"avis" || brand == b"mif1" || brand == b"msf1" {
            return Some("image/avif");
        }
    }
    // ISO BMFF container signature for JPEG XL (00 00 00 0C 'JXL ' ...),
    // or the bare codestream signature.
    if bytes.starts_with(&[0xFF, 0x0A])
        || (bytes.get(4..8) == Some(&b"JXL "[..]) && bytes.get(0..4) == Some(&[0, 0, 0, 0x0C][..]))
    {
        return Some("image/jxl");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};

    fn write_test_png(path: &Path, width: u32, height: u32) {
        let image = RgbaImage::from_pixel(width, height, Rgba([20, 40, 80, 255]));
        image.save_with_format(path, ImageFormat::Png).unwrap();
    }

    #[test]
    fn preview_is_a_jpeg_fitted_inside_1024_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("large.png");
        write_test_png(&source, 1200, 800);

        let rendered = render_photo_image(&source, false).unwrap();
        assert_eq!(rendered.mime_type, "image/jpeg");
        assert!(rendered.bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
        let decoded = image::load_from_memory(&rendered.bytes).unwrap();
        assert_eq!(decoded.width().max(decoded.height()), 1024);
    }

    #[test]
    fn original_returns_the_exact_file_bytes_and_mime() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.png");
        write_test_png(&source, 1200, 800);
        let expected = std::fs::read(&source).unwrap();

        let rendered = render_photo_image(&source, true).unwrap();
        assert_eq!(rendered.mime_type, "image/png");
        assert_eq!(rendered.bytes, expected);
    }

    #[test]
    fn original_mime_falls_back_to_the_file_signature() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("extensionless");
        write_test_png(&source, 16, 10);

        let rendered = render_photo_image(&source, true).unwrap();
        assert_eq!(rendered.mime_type, "image/png");
    }

    #[test]
    fn missing_source_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(render_photo_image(&dir.path().join("missing.png"), false).is_err());
        assert!(render_photo_image(&dir.path().join("missing.png"), true).is_err());
    }
}

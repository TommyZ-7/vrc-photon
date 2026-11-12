//! BC7 encoding for thumbnail canvases.
//!
//! Thumbnails are uploaded to a `Texture2DArray` exactly as they are stored, so
//! the file contains raw BC7 blocks rather than an image container. The canvas
//! dimensions are always multiples of four; the encoder is Intel's ISPC
//! compressor through `intel_tex_2`.

use crate::error::{CoreError, Result};

/// The exact number of BC7 bytes for an image of `width` × `height`.
///
/// BC7 stores 16 bytes per 4×4 block, i.e. one byte per pixel. The dimensions
/// must be positive multiples of four because they are uploaded directly to a
/// GPU texture.
pub fn encoded_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || !width.is_multiple_of(4) || !height.is_multiple_of(4) {
        return None;
    }
    let blocks = usize::try_from(width / 4)
        .ok()?
        .checked_mul(usize::try_from(height / 4).ok()?)?;
    blocks.checked_mul(16)
}

/// Encodes tightly packed RGBA8 pixels into raw BC7 blocks in row order.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let expected = encoded_len(width, height).ok_or_else(|| {
        CoreError::Invalid(format!(
            "BC7 dimensions must be positive multiples of four: {width}x{height}"
        ))
    })?;
    let pixels = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CoreError::Invalid("BC7 image size overflow".to_string()))?;
    if rgba.len() != pixels {
        return Err(CoreError::Invalid(format!(
            "BC7 input must hold {pixels} RGBA bytes, got {}",
            rgba.len()
        )));
    }

    let surface = intel_tex_2::RgbaSurface {
        data: rgba,
        width,
        height,
        stride: width * 4,
    };
    let blocks =
        intel_tex_2::bc7::compress_blocks(&intel_tex_2::bc7::opaque_basic_settings(), &surface);
    if blocks.len() != expected {
        return Err(CoreError::Invalid(format!(
            "BC7 encoder returned {} bytes, expected {expected}",
            blocks.len()
        )));
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc7_length_is_one_byte_per_pixel() {
        assert_eq!(encoded_len(256, 144), Some(256 * 144));
        assert_eq!(encoded_len(384, 216), Some(384 * 216));
        assert_eq!(encoded_len(512, 288), Some(512 * 288));
    }

    #[test]
    fn bc7_requires_positive_multiples_of_four() {
        assert_eq!(encoded_len(0, 144), None);
        assert_eq!(encoded_len(383, 216), None);
        assert_eq!(encoded_len(384, 217), None);
    }

    #[test]
    fn encode_rejects_a_short_rgba_buffer() {
        let error = encode_rgba(8, 8, &[0; 8 * 8 * 4 - 1]).unwrap_err();
        assert!(error.to_string().contains("RGBA bytes"));
    }

    #[test]
    fn encode_returns_the_expected_block_count() {
        let rgba = vec![0u8; 8 * 8 * 4];
        let blocks = encode_rgba(8, 8, &rgba).expect("encoded");
        assert_eq!(blocks.len(), 8 * 8);
    }

    #[test]
    fn encode_rejects_invalid_dimensions() {
        assert!(encode_rgba(7, 8, &[]).is_err());
    }
}

//! Central image decoding with per-decode resource limits.
//!
//! PNG/JPEG/WebP are decoded through the `image` crate with explicit
//! [`image::Limits`]: the decoded pixel count and the allocation budget are
//! bounded before pixel data is allocated. AVIF and JPEG XL use pure-Rust
//! decoders (libjxl is only linked for encoding by `jpegxl-rs`) and reject
//! oversized files from their metadata before reading them into memory.
//!
//! Formats decoded through the `image` crate are returned with their EXIF
//! orientation applied, so thumbnails, full images, compression, and tagging
//! all see the displayed layout. AVIF and JPEG XL keep their own decoders'
//! container/bitstream orientation handling and are not rotated a second time.
//!
//! # Memory bounds
//!
//! Limits are enforced per decode; there is no global semaphore. Several
//! concurrent decodes can therefore each hold their own allocation budget,
//! so the worst-case transient memory is `concurrency × budget`. Callers are
//! expected to keep decode concurrency bounded (the daemon uses a small
//! worker pool), and the per-decode caps keep a single malicious file from
//! consuming unbounded memory.

use std::fs;
use std::path::Path;

use image::metadata::Orientation;
use image::{
    ColorType, DynamicImage, GenericImageView, ImageBuffer, ImageDecoder, ImageReader, RgbaImage,
    imageops::{self, FilterType},
};
use jpegxl_rs::decode::{Event, Events};
use jpegxl_rs::image::ToDynamic;

/// Maximum decoded pixel count (width × height). 64 MP covers 8K (33 MP)
/// with room for large panorama screenshots while staying well below the
/// allocation budget for 8-bit RGBA.
pub const MAX_DECODE_PIXELS: u64 = 64 * 1024 * 1024;
/// Maximum allocation budget for a single `image`-crate decode.
pub const MAX_DECODE_ALLOC_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum compressed file size read for the AVIF/JXL decoders.
pub const MAX_COMPRESSED_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Longest edge kept for the memory-only Quick Look image.
pub const FULL_IMAGE_MAX: u32 = 8192;

/// Decodes a source or derivative image in any format the library can produce.
///
/// PNG/JPEG/WebP/TIFF go through the `image` crate. AVIF and JPEG XL use
/// pure-Rust decoders because libjxl is only linked for encoding by
/// `jpegxl-rs`. All formats are bounded by [`MAX_DECODE_PIXELS`], and any EXIF
/// orientation is applied exactly once.
pub fn decode_image(path: &Path) -> Option<DynamicImage> {
    let extension = normalized_extension(path);

    match extension.as_deref() {
        Some("avif") => decode_avif(path),
        Some("jxl") => decode_jxl(path),
        _ => decode_standard(path),
    }
}

/// Decodes the display file for Quick Look without the thumbnail cap.
///
/// Images under [`FULL_IMAGE_MAX`] keep their original pixels; larger ones are
/// downscaled once so the memory ceiling is bounded. Formats decoded by the
/// `image` crate are expanded into one pre-sized RGBA allocation, so a full RGB
/// source and a full RGBA conversion never exist at the same time.
pub fn decode_full_image(source: &Path) -> Option<RgbaImage> {
    match normalized_extension(source).as_deref() {
        Some("avif" | "jxl") => decode_full_fallback(source),
        _ => decode_standard_rgba(source).or_else(|| decode_full_fallback(source)),
    }
}

fn normalized_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

fn decode_full_fallback(source: &Path) -> Option<RgbaImage> {
    let image = decode_image(source)?;
    let (width, height) = image.dimensions();
    if width.max(height) <= FULL_IMAGE_MAX {
        Some(image.into_rgba8())
    } else {
        let scaled = image.thumbnail(FULL_IMAGE_MAX, FULL_IMAGE_MAX);
        drop(image);
        Some(scaled.into_rgba8())
    }
}

/// Decodes a thumbnail-sized image without expanding the full source.
///
/// JPEGs use the decoder's IDCT scaling (1/2, 1/4, 1/8) so a large photo is
/// never allocated at full resolution. Other formats decode normally and are
/// scaled with the same fast integer filter as before.
pub(crate) fn decode_thumbnail_image(source: &Path, max_size: u32) -> Option<RgbaImage> {
    if matches!(
        normalized_extension(source).as_deref(),
        Some("jpg" | "jpeg")
    ) && let Some(image) = decode_jpeg_scaled(source, max_size)
    {
        let orientation = jpeg_orientation(source);
        return Some(finish_thumbnail_image(image, orientation, max_size));
    }
    let image = decode_image(source)?.into_rgba8();
    Some(finish_thumbnail_image(
        image,
        Orientation::NoTransforms,
        max_size,
    ))
}

/// Decodes a thumbnail source and renders it onto a fixed landscape canvas.
///
/// The source is decoded with its EXIF orientation applied. A portrait image
/// is then rotated 90° counter-clockwise so the stored canvas is landscape;
/// `rotated` tells the display which UV transform restores it. The result is
/// stretched to exactly `width` × `height` without cropping or padding.
pub(crate) fn decode_thumbnail_canvas(
    source: &Path,
    width: u32,
    height: u32,
) -> Option<(RgbaImage, bool)> {
    let image = decode_thumbnail_image(source, width.max(height))?;
    let rotated = image.height() > image.width();
    let image = if rotated {
        imageops::rotate270(&image)
    } else {
        image
    };
    Some((
        imageops::resize(&image, width, height, FilterType::Triangle),
        rotated,
    ))
}

/// Decodes a JPEG with the smallest IDCT scale that still covers `max_size`.
///
/// The returned pixels are still in the stored orientation; the caller applies
/// EXIF orientation before the final resize. CMYK and 16-bit JPEGs fall back
/// to the full decoder because their pixel format needs a color conversion
/// this path does not implement.
fn decode_jpeg_scaled(source: &Path, max_size: u32) -> Option<RgbaImage> {
    let file = fs::File::open(source).ok()?;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
    decoder.set_max_decoding_buffer_size(MAX_DECODE_ALLOC_BYTES as usize);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    if !within_pixel_limit(width, height) {
        return reject_too_many_pixels(source, width, height);
    }
    let requested = max_size.min(u32::from(u16::MAX)) as u16;
    decoder.scale(requested, requested).ok()?;
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => expand_l8_to_rgba(pixels, width, height)?,
        jpeg_decoder::PixelFormat::RGB24 => expand_rgb24_to_rgba(pixels, width, height)?,
        _ => return None,
    };
    RgbaImage::from_raw(width, height, rgba)
}

fn jpeg_orientation(source: &Path) -> Orientation {
    open_decoder(source)
        .and_then(|mut decoder| decoder.orientation().ok())
        .unwrap_or(Orientation::NoTransforms)
}

fn expand_l8_to_rgba(pixels: Vec<u8>, width: u32, height: u32) -> Option<Vec<u8>> {
    let count = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    if pixels.len() != count {
        return None;
    }
    let mut rgba = Vec::with_capacity(count.checked_mul(4)?);
    for value in pixels {
        rgba.extend_from_slice(&[value, value, value, 255]);
    }
    Some(rgba)
}

fn expand_rgb24_to_rgba(pixels: Vec<u8>, width: u32, height: u32) -> Option<Vec<u8>> {
    let count = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    if pixels.len() != count.checked_mul(3)? {
        return None;
    }
    let mut rgba = Vec::with_capacity(count.checked_mul(4)?);
    for pixel in pixels.as_chunks::<3>().0 {
        rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
    Some(rgba)
}

/// Applies EXIF orientation and fits the result into a square thumbnail box.
///
/// Unlike [`finish_full_image`], this matches the old thumbnail behavior of
/// scaling small sources up to the box as well.
fn finish_thumbnail_image(image: RgbaImage, orientation: Orientation, max_size: u32) -> RgbaImage {
    let mut image = DynamicImage::ImageRgba8(image);
    image.apply_orientation(orientation);
    image.thumbnail(max_size, max_size).into_rgba8()
}

/// Decodes an 8-bit image through one allocation sized for RGBA.
///
/// The decoder writes its native bytes into the spare capacity, then the
/// buffer is expanded in place from the last pixel backwards, so the source
/// and the converted pixels share one full-size allocation. Sixteen-bit and
/// float formats keep the image-crate conversion path.
fn decode_standard_rgba(source: &Path) -> Option<RgbaImage> {
    let mut reader = ImageReader::open(source).ok()?.with_guessed_format().ok()?;
    reader.limits(decode_limits());
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if !within_pixel_limit(width, height) {
        return reject_too_many_pixels(source, width, height);
    }
    let color = decoder.color_type();
    if !matches!(
        color,
        ColorType::Rgba8 | ColorType::Rgb8 | ColorType::L8 | ColorType::La8
    ) {
        return None;
    }
    let pixels = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let rgba_len = pixels.checked_mul(4)?;
    let decoded_len = usize::try_from(decoder.total_bytes()).ok()?;
    if decoded_len > rgba_len {
        return None;
    }
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut buffer = Vec::with_capacity(rgba_len);
    buffer.resize(decoded_len, 0);
    decoder.read_image(&mut buffer).ok()?;
    expand_decoded_buffer(&mut buffer, color, pixels, rgba_len)?;
    let image = RgbaImage::from_raw(width, height, buffer)?;
    Some(finish_full_image(image, orientation))
}

/// Expands an 8-bit decoded buffer to RGBA in place.
fn expand_decoded_buffer(
    buffer: &mut Vec<u8>,
    color: ColorType,
    pixels: usize,
    rgba_len: usize,
) -> Option<()> {
    debug_assert!(
        buffer.capacity() >= rgba_len,
        "decoded buffer must reserve the RGBA size before expansion"
    );
    match color {
        ColorType::Rgba8 => {
            if buffer.len() != rgba_len {
                return None;
            }
            Some(())
        }
        ColorType::Rgb8 => {
            if buffer.len() != pixels.checked_mul(3)? {
                return None;
            }
            buffer.resize(rgba_len, 0);
            for index in (0..pixels).rev() {
                let src = index * 3;
                let dst = index * 4;
                let pixel = [buffer[src], buffer[src + 1], buffer[src + 2]];
                buffer[dst..dst + 4].copy_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
            Some(())
        }
        ColorType::L8 => {
            if buffer.len() != pixels {
                return None;
            }
            buffer.resize(rgba_len, 0);
            for index in (0..pixels).rev() {
                let value = buffer[index];
                let dst = index * 4;
                buffer[dst..dst + 4].copy_from_slice(&[value, value, value, 255]);
            }
            Some(())
        }
        ColorType::La8 => {
            if buffer.len() != pixels.checked_mul(2)? {
                return None;
            }
            buffer.resize(rgba_len, 0);
            for index in (0..pixels).rev() {
                let src = index * 2;
                let value = buffer[src];
                let alpha = buffer[src + 1];
                let dst = index * 4;
                buffer[dst..dst + 4].copy_from_slice(&[value, value, value, alpha]);
            }
            Some(())
        }
        _ => None,
    }
}

/// Applies EXIF orientation before the one downscale, matching the order used
/// by the image-crate path.
fn finish_full_image(image: RgbaImage, orientation: Orientation) -> RgbaImage {
    let mut image = DynamicImage::ImageRgba8(image);
    image.apply_orientation(orientation);
    let rgba = image.into_rgba8();
    if rgba.width().max(rgba.height()) <= FULL_IMAGE_MAX {
        rgba
    } else {
        DynamicImage::ImageRgba8(rgba)
            .thumbnail(FULL_IMAGE_MAX, FULL_IMAGE_MAX)
            .into_rgba8()
    }
}

/// Displayed dimensions and the EXIF orientation read from the file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub orientation: Orientation,
}

/// Reads displayed image dimensions and EXIF orientation without decoding
/// pixel data.
///
/// 90/270-degree orientations swap the stored width and height, so `width`
/// and `height` match what thumbnails and full images display while
/// `orientation` lets callers account for the extra rotation buffer.
pub fn image_info(path: &Path) -> Option<ImageInfo> {
    let mut decoder = open_decoder(path)?;
    let (width, height) = decoder.dimensions();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (width, height) = oriented_dimensions(width, height, orientation);
    Some(ImageInfo {
        width,
        height,
        orientation,
    })
}

/// Reads displayed image dimensions without decoding pixel data.
///
/// EXIF orientation is read from the file header and 90/270-degree
/// orientations swap the stored width and height. This keeps the scanner's
/// width/height semantics equal to what thumbnails and full images display.
pub fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    image_info(path).map(|info| (info.width, info.height))
}

/// Returns the displayed dimensions for a raw size and EXIF orientation.
pub fn oriented_dimensions(width: u32, height: u32, orientation: Orientation) -> (u32, u32) {
    if matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    ) {
        (height, width)
    } else {
        (width, height)
    }
}

fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::no_limits();
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    limits
}

fn open_decoder(path: &Path) -> Option<impl ImageDecoder> {
    let mut reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    reader.limits(decode_limits());
    reader.into_decoder().ok()
}

fn within_pixel_limit(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && u64::from(width).saturating_mul(u64::from(height)) <= MAX_DECODE_PIXELS
}

/// Rejects an image whose header declares more pixels than the cap allows.
/// The check happens before any full-size pixel buffer is allocated.
fn reject_too_many_pixels<T>(path: &Path, width: u32, height: u32) -> Option<T> {
    tracing::warn!(
        path = %path.display(),
        width,
        height,
        pixels = u64::from(width).saturating_mul(u64::from(height)),
        limit = MAX_DECODE_PIXELS,
        "refusing to decode image over the pixel limit"
    );
    None
}

fn decode_standard(path: &Path) -> Option<DynamicImage> {
    let mut decoder = open_decoder(path)?;
    let (width, height) = decoder.dimensions();
    if !within_pixel_limit(width, height) {
        return reject_too_many_pixels(path, width, height);
    }

    // Read the EXIF orientation while the decoder only has header data.
    // AVIF/JXL are handled separately because their pure-Rust decoders do
    // not expose EXIF orientation here.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);

    // Mirror `ImageReader::decode`: reserve the output buffer against the
    // allocation budget before handing the decoder to `DynamicImage`.
    let mut limits = decode_limits();
    if let Err(error) = limits.reserve(decoder.total_bytes()) {
        tracing::warn!(
            path = %path.display(),
            width,
            height,
            limit = MAX_DECODE_ALLOC_BYTES,
            %error,
            "refusing to decode image over the allocation limit"
        );
        return None;
    }
    if let Err(error) = decoder.set_limits(limits) {
        tracing::warn!(
            path = %path.display(),
            width,
            height,
            %error,
            "refusing to decode image that exceeds decoder limits"
        );
        return None;
    }

    let mut image = match DynamicImage::from_decoder(decoder) {
        Ok(image) => image,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "cannot decode image"
            );
            return None;
        }
    };
    image.apply_orientation(orientation);
    Some(image)
}

fn decode_avif(path: &Path) -> Option<DynamicImage> {
    let data = read_compressed(path)?;
    if !oxideav_avif::probe(&data) {
        tracing::warn!(path = %path.display(), "cannot decode AVIF: probe failed");
        return None;
    }
    let options = oxideav_avif::DecodeOptions::default()
        .with_max_pixels(Some(MAX_DECODE_PIXELS))
        .with_max_bytes(Some(MAX_COMPRESSED_FILE_BYTES));
    let image = match oxideav_avif::decode_with(&data, &options) {
        Ok(image) => image,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "cannot decode AVIF"
            );
            return None;
        }
    };
    let width = image.width();
    let height = image.height();
    if !within_pixel_limit(width, height) {
        return reject_too_many_pixels(path, width, height);
    }
    ImageBuffer::from_raw(width, height, image.to_rgba8()).map(DynamicImage::ImageRgba8)
}

fn decode_jxl(path: &Path) -> Option<DynamicImage> {
    let data = read_compressed(path)?;
    let mut decoder = match jpegxl_rs::decoder_builder().build() {
        Ok(decoder) => decoder,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "cannot initialize JPEG XL decoder"
            );
            return None;
        }
    };
    if let Some((width, height)) = jxl_dimensions(&mut decoder, &data)
        && !within_pixel_limit(width, height)
    {
        return reject_too_many_pixels(path, width, height);
    }
    match decoder.decode_to_image(&data) {
        Ok(Some(image)) => Some(image),
        Ok(None) => {
            tracing::warn!(
                path = %path.display(),
                "JPEG XL image is not representable as a DynamicImage"
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "cannot decode JPEG XL"
            );
            None
        }
    }
}

/// Reads the JPEG XL header without allocating the output image.
fn jxl_dimensions(
    decoder: &mut jpegxl_rs::decode::JxlDecoder<'_, '_>,
    data: &[u8],
) -> Option<(u32, u32)> {
    let mut session = decoder.session(Events::empty()).ok()?;
    let mut input = data;
    loop {
        match session.process(&mut input).ok()? {
            Event::BasicInfo(info) => return Some((info.xsize, info.ysize)),
            Event::Success | Event::NeedMoreInput => return None,
            _ => {}
        }
    }
}

fn read_compressed(path: &Path) -> Option<Vec<u8>> {
    let bytes = fs::metadata(path).ok()?.len();
    if bytes > MAX_COMPRESSED_FILE_BYTES {
        tracing::warn!(
            path = %path.display(),
            bytes,
            limit = MAX_COMPRESSED_FILE_BYTES,
            "refusing to read compressed image over the file-size limit"
        );
        return None;
    }
    fs::read(path).ok()
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;
    use std::path::Path;

    use image::{ImageEncoder as _, RgbaImage};

    /// Builds a little-endian TIFF header with a single IFD0 orientation entry.
    pub(crate) fn exif_orientation(value: u16) -> Vec<u8> {
        let mut exif = Vec::new();
        exif.extend_from_slice(b"II\x2a\x00");
        exif.extend_from_slice(&8u32.to_le_bytes());
        exif.extend_from_slice(&1u16.to_le_bytes());
        exif.extend_from_slice(&0x0112u16.to_le_bytes());
        exif.extend_from_slice(&3u16.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        exif.extend_from_slice(&value.to_le_bytes());
        exif.extend_from_slice(&0u16.to_le_bytes());
        exif.extend_from_slice(&0u32.to_le_bytes());
        exif
    }

    /// Writes a JPEG carrying one EXIF orientation value.
    pub(crate) fn write_jpeg_with_orientation(path: &Path, image: &RgbaImage, value: u16) {
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100);
        encoder
            .set_exif_metadata(exif_orientation(value))
            .expect("EXIF metadata");
        encoder.encode_image(image).expect("JPEG encode");
        fs::write(path, bytes).expect("write JPEG");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor;
    use crate::model::{CompressionFormat, CompressionProfile};
    use crate::thumbnail::{self, ThumbnailSpec};
    use image::{GenericImageView, Rgb, RgbImage, Rgba, RgbaImage};
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn write_source(dir: &Path) -> PathBuf {
        let path = dir.join("VRChat_2026-10-21_21-14-03.221_4x3.png");
        RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();
        path
    }

    fn profile(format: CompressionFormat) -> CompressionProfile {
        CompressionProfile {
            format,
            quality: 80,
            ..CompressionProfile::default()
        }
    }

    #[test]
    fn decodes_png_and_webp() {
        let dir = tempdir().unwrap();
        let source = write_source(dir.path());
        assert_eq!(decode_image(&source).unwrap().dimensions(), (4, 3));

        let result =
            compressor::compress_image(&source, &profile(CompressionFormat::Webp)).unwrap();
        assert_eq!(
            decode_image(&result.output_path).unwrap().dimensions(),
            (4, 3)
        );
    }

    #[test]
    fn decodes_jpeg() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.jpg");
        RgbImage::from_pixel(4, 3, Rgb([10, 20, 30]))
            .save(&source)
            .unwrap();

        assert_eq!(decode_image(&source).unwrap().dimensions(), (4, 3));
    }

    fn write_rgb_jpeg(path: &Path, width: u32, height: u32) {
        RgbImage::from_fn(width, height, |x, y| {
            Rgb([
                (x * 7 % 251) as u8,
                (y * 11 % 241) as u8,
                ((x + y) * 13 % 239) as u8,
            ])
        })
        .save(path)
        .unwrap();
    }

    fn write_rgb_png(path: &Path, width: u32, height: u32) {
        RgbImage::from_fn(width, height, |x, y| {
            Rgb([
                (x * 7 % 251) as u8,
                (y * 11 % 241) as u8,
                ((x + y) * 13 % 239) as u8,
            ])
        })
        .save(path)
        .unwrap();
    }

    #[test]
    fn full_image_matches_the_standard_decode_for_rgb_jpeg() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("rgb.jpg");
        write_rgb_jpeg(&source, 37, 21);

        let expected = decode_image(&source).expect("standard decode").into_rgba8();
        let actual = decode_full_image(&source).expect("full decode");

        assert_eq!(actual.dimensions(), (37, 21));
        assert_eq!(actual.as_raw(), expected.as_raw());
    }

    #[test]
    fn full_image_matches_the_standard_decode_for_rgb_png() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("rgb.png");
        write_rgb_png(&source, 37, 21);

        let expected = decode_image(&source).expect("standard decode").into_rgba8();
        let actual = decode_full_image(&source).expect("full decode");

        assert_eq!(actual.dimensions(), (37, 21));
        assert_eq!(actual.as_raw(), expected.as_raw());
    }

    #[test]
    fn full_image_matches_the_standard_decode_for_rgba_png() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("rgba.png");
        RgbaImage::from_fn(37, 21, |x, y| {
            Rgba([
                (x * 5 % 251) as u8,
                (y * 3 % 241) as u8,
                ((x + y) * 17 % 239) as u8,
                (x * 29 % 256) as u8,
            ])
        })
        .save(&source)
        .unwrap();

        let expected = decode_image(&source).expect("standard decode").into_rgba8();
        let actual = decode_full_image(&source).expect("full decode");

        assert_eq!(actual.dimensions(), (37, 21));
        assert_eq!(actual.as_raw(), expected.as_raw());
    }

    #[test]
    fn full_image_matches_the_standard_decode_for_grayscale_png() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("gray.png");
        image::GrayImage::from_fn(37, 21, |x, y| image::Luma([(x * 7 + y * 11) as u8]))
            .save(&source)
            .unwrap();

        let expected = decode_image(&source).expect("standard decode").into_rgba8();
        let actual = decode_full_image(&source).expect("full decode");

        assert_eq!(actual.dimensions(), (37, 21));
        assert_eq!(actual.as_raw(), expected.as_raw());
    }

    #[test]
    fn full_image_matches_the_standard_decode_for_grayscale_alpha_png() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("gray-alpha.png");
        image::GrayAlphaImage::from_fn(37, 21, |x, y| {
            image::LumaA([(x * 7 + y * 11) as u8, (x * 13 % 256) as u8])
        })
        .save(&source)
        .unwrap();

        let expected = decode_image(&source).expect("standard decode").into_rgba8();
        let actual = decode_full_image(&source).expect("full decode");

        assert_eq!(actual.dimensions(), (37, 21));
        assert_eq!(actual.as_raw(), expected.as_raw());
    }

    #[test]
    fn full_image_falls_back_for_16_bit_png() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("gray16.png");
        image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_fn(37, 21, |x, y| {
            image::Luma([(x * 1000 + y * 37) as u16])
        })
        .save(&source)
        .unwrap();

        let expected = decode_image(&source).expect("standard decode").into_rgba8();
        let actual = decode_full_image(&source).expect("full decode");

        assert_eq!(actual.dimensions(), (37, 21));
        assert_eq!(actual.as_raw(), expected.as_raw());
    }

    #[test]
    fn full_image_matches_the_standard_decode_for_webp() {
        let dir = tempdir().unwrap();
        let source = write_source(dir.path());
        let compressed =
            compressor::compress_image(&source, &profile(CompressionFormat::Webp)).unwrap();

        let expected = decode_image(&compressed.output_path)
            .expect("standard decode")
            .into_rgba8();
        let actual = decode_full_image(&compressed.output_path).expect("full decode");

        assert_eq!(actual.as_raw(), expected.as_raw());
    }

    #[test]
    fn full_image_caps_only_the_long_edge() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("wide.png");
        write_rgb_png(&source, FULL_IMAGE_MAX + 16, 32);

        let expected = decode_image(&source)
            .expect("standard decode")
            .thumbnail(FULL_IMAGE_MAX, FULL_IMAGE_MAX)
            .into_rgba8();
        let actual = decode_full_image(&source).expect("full decode");

        assert_eq!(actual.dimensions(), (FULL_IMAGE_MAX, 32));
        assert_eq!(actual.as_raw(), expected.as_raw());
    }

    #[test]
    fn decodes_avif() {
        let dir = tempdir().unwrap();
        let source = write_source(dir.path());
        let result =
            compressor::compress_image(&source, &profile(CompressionFormat::Avif)).unwrap();
        assert_eq!(
            decode_image(&result.output_path).unwrap().dimensions(),
            (4, 3)
        );
    }

    #[test]
    fn decodes_jpeg_xl() {
        let dir = tempdir().unwrap();
        let source = write_source(dir.path());
        let mut jxl = profile(CompressionFormat::JpegXl);
        jxl.lossless = true;
        let result = compressor::compress_image(&source, &jxl).unwrap();
        assert_eq!(
            decode_image(&result.output_path).unwrap().dimensions(),
            (4, 3)
        );

        let data = fs::read(&result.output_path).unwrap();
        let mut decoder = jpegxl_rs::decoder_builder().build().unwrap();
        assert_eq!(jxl_dimensions(&mut decoder, &data), Some((4, 3)));
    }

    #[test]
    fn rejects_an_image_declared_over_the_pixel_cap() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("huge.png");
        write_png_with_declared_dimensions(&path, 10_000, 7_000);

        assert!(decode_image(&path).is_none());
    }

    #[test]
    fn rejects_oversized_avif_and_jxl_before_reading() {
        let dir = tempdir().unwrap();
        for name in ["huge.avif", "huge.jxl"] {
            let path = dir.path().join(name);
            let file = fs::File::create(&path).unwrap();
            file.set_len(MAX_COMPRESSED_FILE_BYTES + 1).unwrap();
            assert!(decode_image(&path).is_none(), "{name} should be rejected");
        }
    }

    #[test]
    fn thumbnail_decode_scales_large_jpegs_before_pixel_decode() {
        let dir = tempdir().unwrap();
        let source = RgbaImage::from_pixel(2048, 1024, Rgba([20, 40, 60, 255]));
        let path = dir.path().join("large.jpg");
        test_support::write_jpeg_with_orientation(&path, &source, 1);

        let scaled = decode_jpeg_scaled(&path, 256).expect("scaled JPEG");
        assert_eq!(
            scaled.dimensions(),
            (256, 128),
            "the 1/8 IDCT output is decoded instead of the full source"
        );

        let thumbnail = decode_thumbnail_image(&path, 256).expect("thumbnail");
        assert_eq!(thumbnail.dimensions(), (256, 128));
    }

    #[test]
    fn decode_applies_exif_orientation_once() {
        let dir = tempdir().unwrap();
        let source = RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });

        for (orientation, expected, pixels) in [
            (1, (2, 1), ['r', 'b']),
            (3, (2, 1), ['b', 'r']),
            (6, (1, 2), ['r', 'b']),
            (8, (1, 2), ['b', 'r']),
        ] {
            let path = dir.path().join(format!("orientation-{orientation}.jpg"));
            test_support::write_jpeg_with_orientation(&path, &source, orientation);

            let image = decode_image(&path).expect("oriented JPEG");
            assert_eq!(image.dimensions(), expected, "orientation {orientation}");
            let rgba = image.into_rgba8();
            let actual: Vec<char> = rgba
                .pixels()
                .map(|pixel| if pixel[0] > pixel[2] { 'r' } else { 'b' })
                .collect();
            assert_eq!(actual, pixels, "orientation {orientation}");

            let full = decode_full_image(&path).expect("oriented full JPEG");
            assert_eq!(
                full.dimensions(),
                expected,
                "full image keeps orientation {orientation}"
            );
            assert_eq!(
                full.as_raw(),
                rgba.as_raw(),
                "full image matches the standard RGBA pixels for orientation {orientation}"
            );
        }
    }

    #[test]
    fn thumbnails_and_compression_use_oriented_dimensions() {
        let dir = tempdir().unwrap();
        let source_image = RgbaImage::from_pixel(400, 200, Rgba([40, 120, 220, 255]));

        for (index, orientation) in [6, 8].into_iter().enumerate() {
            let source = dir.path().join(format!("rotated-{orientation}.jpg"));
            test_support::write_jpeg_with_orientation(&source, &source_image, orientation);

            assert_eq!(
                decode_image(&source).unwrap().dimensions(),
                (200, 400),
                "decode applies orientation {orientation}"
            );

            let thumbnail = thumbnail::ensure_thumbnail_in(
                index as i64 + 1,
                &source,
                &dir.path().join("thumbnails"),
                ThumbnailSpec::new(384),
            )
            .unwrap()
            .expect("thumbnail");
            assert_eq!(
                fs::metadata(&thumbnail).unwrap().len(),
                384 * 216,
                "thumbnail uses the landscape canvas after orientation {orientation}"
            );

            let compressed =
                compressor::compress_image(&source, &profile(CompressionFormat::Webp)).unwrap();
            assert_eq!(
                (compressed.width, compressed.height),
                (200, 400),
                "compression reports displayed dimensions for orientation {orientation}"
            );
            assert_eq!(
                decode_image(&compressed.output_path).unwrap().dimensions(),
                (200, 400)
            );
        }
    }

    #[test]
    fn image_info_reports_oriented_dimensions_and_orientation() {
        let dir = tempdir().unwrap();
        let source = RgbaImage::from_pixel(400, 200, Rgba([40, 120, 220, 255]));
        let path = dir.path().join("rotated.jpg");
        test_support::write_jpeg_with_orientation(&path, &source, 6);

        let info = image_info(&path).expect("image info");

        assert_eq!((info.width, info.height), (200, 400));
        assert_eq!(info.orientation, Orientation::Rotate90);
    }

    fn write_png_with_declared_dimensions(path: &Path, width: u32, height: u32) {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 0, 0, 255]).unwrap();
        }
        // The signature is 8 bytes; IHDR data starts at 16 (length + type).
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        fs::write(path, bytes).unwrap();
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }
}

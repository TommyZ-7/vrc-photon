//! A decoded full image uploaded as a single-layer RGBA8 texture array.
//!
//! The handle owns the array for as long as any paint closure holds the
//! `Arc`; the last drop releases the GPU memory. Callers must not copy the
//! array out and keep it past the handle.

use std::sync::Arc;

use gpui_kit::{DevicePixels, TextureArray, TextureArrayFormat, Window, size};

/// A full image resident in a one-layer `Rgba8Unorm` texture array.
pub(crate) struct FullImageTexture {
    array: TextureArray,
    dimensions: (u32, u32),
}

impl FullImageTexture {
    /// Creates a one-layer RGBA8 array and uploads `rgba` into it.
    ///
    /// # Errors
    /// Returns an error when the image has a zero dimension, a dimension does
    /// not fit in an `i32`, or the backend cannot create or upload the array.
    /// The array is released before an upload error is returned.
    pub(crate) fn new(window: &mut Window, rgba: &image::RgbaImage) -> anyhow::Result<Arc<Self>> {
        let (width, height) = rgba.dimensions();
        if width == 0 || height == 0 {
            anyhow::bail!("full image is empty ({width}x{height})");
        }
        let pixel_width = i32::try_from(width)
            .map_err(|_| anyhow::anyhow!("full image width {width} does not fit in an i32"))?;
        let pixel_height = i32::try_from(height)
            .map_err(|_| anyhow::anyhow!("full image height {height} does not fit in an i32"))?;
        let array = window.create_texture_array(
            size(DevicePixels(pixel_width), DevicePixels(pixel_height)),
            1,
            TextureArrayFormat::Rgba8Unorm,
        )?;
        if let Err(error) = window.upload_texture_array_layer(array, 0, rgba.as_raw()) {
            array.remove();
            return Err(error);
        }
        Ok(Arc::new(Self {
            array,
            dimensions: (width, height),
        }))
    }

    /// The decoded pixel size of the uploaded image.
    pub(crate) fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }

    /// The one-layer array to paint. Held `Copy` handles stay valid only while
    /// the owning `Arc` is alive.
    pub(crate) fn array(&self) -> TextureArray {
        self.array
    }
}

impl Drop for FullImageTexture {
    fn drop(&mut self) {
        self.array.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{DevicePixels, size};

    fn test_texture(width: u32, height: u32) -> Arc<FullImageTexture> {
        Arc::new(FullImageTexture {
            array: TextureArray::new_for_test(
                size(DevicePixels(width as i32), DevicePixels(height as i32)),
                1,
            ),
            dimensions: (width, height),
        })
    }

    #[test]
    fn dimensions_are_the_decoded_size() {
        assert_eq!(test_texture(3, 2).dimensions(), (3, 2));
    }

    #[test]
    fn dropping_a_test_texture_is_quiet() {
        drop(test_texture(3, 2));
    }
}

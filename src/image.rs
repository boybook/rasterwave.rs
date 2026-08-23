use crate::{Error, Result};

/// One eight-bit RGB pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Rgb {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Rgb {
    /// Construct an RGB pixel.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Owned row-major RGB image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbImage {
    width: u32,
    height: u32,
    pixels: Vec<Rgb>,
}

impl RgbImage {
    /// Construct an image after validating its buffer size.
    pub fn new(width: u32, height: u32, pixels: Vec<Rgb>) -> Result<Self> {
        let expected = pixel_count(width, height);
        if pixels.len() != expected {
            return Err(Error::InvalidImageBuffer {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Construct from interleaved `R,G,B` bytes.
    pub fn from_rgb8(width: u32, height: u32, bytes: &[u8]) -> Result<Self> {
        let expected = pixel_count(width, height).saturating_mul(3);
        if bytes.len() != expected {
            return Err(Error::InvalidRgbByteBuffer {
                expected,
                actual: bytes.len(),
            });
        }
        let pixels = bytes
            .chunks_exact(3)
            .map(|pixel| Rgb::new(pixel[0], pixel[1], pixel[2]))
            .collect();
        Self::new(width, height, pixels)
    }

    /// Construct a solid-color image.
    pub fn filled(width: u32, height: u32, pixel: Rgb) -> Self {
        Self {
            width,
            height,
            pixels: vec![pixel; pixel_count(width, height)],
        }
    }

    /// Image width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Row-major pixel buffer.
    pub fn pixels(&self) -> &[Rgb] {
        &self.pixels
    }

    /// Mutable row-major pixel buffer.
    pub fn pixels_mut(&mut self) -> &mut [Rgb] {
        &mut self.pixels
    }

    /// Copy pixels into interleaved `R,G,B` bytes.
    pub fn to_rgb8(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.pixels.len() * 3);
        for pixel in &self.pixels {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
        bytes
    }

    /// Consume the image and return its row-major pixel backing.
    pub fn into_pixels(self) -> Vec<Rgb> {
        self.pixels
    }

    /// Return one row, or `None` when `row` is outside the image.
    pub fn row(&self, row: u32) -> Option<&[Rgb]> {
        if row >= self.height {
            return None;
        }
        let width = self.width as usize;
        let start = row as usize * width;
        Some(&self.pixels[start..start + width])
    }

    /// Resize with nearest-neighbor sampling.
    ///
    /// This intentionally deterministic helper is useful for codec frontends;
    /// applications may use a higher-quality image library before construction.
    /// Resizing an empty source to a non-empty target produces black RGB values
    /// rather than indexing an absent source pixel.
    pub fn resize_nearest(&self, width: u32, height: u32) -> Self {
        if width == self.width && height == self.height {
            return self.clone();
        }
        if self.width == 0 || self.height == 0 {
            return Self::filled(width, height, Rgb::default());
        }
        let mut output = Vec::with_capacity(pixel_count(width, height));
        for y in 0..height {
            let source_y = (u64::from(y) * u64::from(self.height) / u64::from(height)) as u32;
            for x in 0..width {
                let source_x = (u64::from(x) * u64::from(self.width) / u64::from(width)) as u32;
                let index = source_y as usize * self.width as usize + source_x as usize;
                output.push(self.pixels[index]);
            }
        }
        Self {
            width,
            height,
            pixels: output,
        }
    }
}

/// Owned row-major eight-bit grayscale image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrayImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl GrayImage {
    /// Construct an image after validating its buffer size.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self> {
        let expected = pixel_count(width, height);
        if pixels.len() != expected {
            return Err(Error::InvalidImageBuffer {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Image width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Row-major luminance buffer.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

fn pixel_count(width: u32, height: u32) -> usize {
    (width as usize).saturating_mul(height as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb8_round_trip_is_exact() {
        let bytes = vec![1, 2, 3, 4, 5, 6];
        let image = RgbImage::from_rgb8(2, 1, &bytes).unwrap();
        assert_eq!(image.to_rgb8(), bytes);
    }

    #[test]
    fn resizing_an_empty_source_is_defined() {
        let empty = RgbImage::new(0, 0, Vec::new()).unwrap();
        assert_eq!(
            empty.resize_nearest(2, 2),
            RgbImage::filled(2, 2, Rgb::default())
        );
    }
}

//! Deterministic image-quality metrics used by interoperability validation.

use crate::{Error, Result, RgbImage};

/// Objective comparison between two RGB images.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageMetrics {
    /// Mean squared error across all RGB components.
    pub mse: f64,
    /// Peak signal-to-noise ratio. Identical images produce infinity.
    pub psnr_db: f64,
    /// Mean 8x8-block luminance structural similarity in approximately
    /// `-1.0..=1.0`.
    pub ssim: f64,
}

/// Compare equal-sized RGB images using MSE, PSNR and block SSIM.
pub fn compare_rgb(reference: &RgbImage, candidate: &RgbImage) -> Result<ImageMetrics> {
    if reference.width() != candidate.width() || reference.height() != candidate.height() {
        return Err(Error::InvalidConfiguration(
            "image metrics require equal dimensions",
        ));
    }
    let mut squared_error = 0.0_f64;
    for (a, b) in reference.pixels().iter().zip(candidate.pixels()) {
        squared_error += (f64::from(a.r) - f64::from(b.r)).powi(2);
        squared_error += (f64::from(a.g) - f64::from(b.g)).powi(2);
        squared_error += (f64::from(a.b) - f64::from(b.b)).powi(2);
    }
    let component_count = reference.pixels().len().saturating_mul(3).max(1);
    let mse = squared_error / component_count as f64;
    let psnr_db = if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0_f64.powi(2) / mse).log10()
    };
    Ok(ImageMetrics {
        mse,
        psnr_db,
        ssim: block_ssim(reference, candidate),
    })
}

fn block_ssim(reference: &RgbImage, candidate: &RgbImage) -> f64 {
    const BLOCK: u32 = 8;
    const C1: f64 = 6.5025;
    const C2: f64 = 58.5225;
    let mut total = 0.0;
    let mut blocks = 0_u64;
    for y0 in (0..reference.height()).step_by(BLOCK as usize) {
        for x0 in (0..reference.width()).step_by(BLOCK as usize) {
            let x1 = (x0 + BLOCK).min(reference.width());
            let y1 = (y0 + BLOCK).min(reference.height());
            let count = u64::from(x1 - x0) * u64::from(y1 - y0);
            if count == 0 {
                continue;
            }
            let mut sum_a = 0.0;
            let mut sum_b = 0.0;
            for y in y0..y1 {
                for x in x0..x1 {
                    let index = y as usize * reference.width() as usize + x as usize;
                    sum_a += luminance(reference.pixels()[index]);
                    sum_b += luminance(candidate.pixels()[index]);
                }
            }
            let mean_a = sum_a / count as f64;
            let mean_b = sum_b / count as f64;
            let mut variance_a = 0.0;
            let mut variance_b = 0.0;
            let mut covariance = 0.0;
            for y in y0..y1 {
                for x in x0..x1 {
                    let index = y as usize * reference.width() as usize + x as usize;
                    let a = luminance(reference.pixels()[index]) - mean_a;
                    let b = luminance(candidate.pixels()[index]) - mean_b;
                    variance_a += a * a;
                    variance_b += b * b;
                    covariance += a * b;
                }
            }
            let denominator = (count.saturating_sub(1)).max(1) as f64;
            variance_a /= denominator;
            variance_b /= denominator;
            covariance /= denominator;
            total += ((2.0 * mean_a * mean_b + C1) * (2.0 * covariance + C2))
                / ((mean_a.powi(2) + mean_b.powi(2) + C1) * (variance_a + variance_b + C2));
            blocks += 1;
        }
    }
    if blocks == 0 {
        1.0
    } else {
        total / blocks as f64
    }
}

#[inline]
fn luminance(pixel: crate::Rgb) -> f64 {
    0.299 * f64::from(pixel.r) + 0.587 * f64::from(pixel.g) + 0.114 * f64::from(pixel.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rgb;

    #[test]
    fn identical_images_have_perfect_metrics() {
        let image = RgbImage::filled(16, 16, Rgb::new(12, 34, 56));
        let metrics = compare_rgb(&image, &image).unwrap();
        assert_eq!(metrics.mse, 0.0);
        assert!(metrics.psnr_db.is_infinite());
        assert!((metrics.ssim - 1.0).abs() < 1e-12);
    }
}

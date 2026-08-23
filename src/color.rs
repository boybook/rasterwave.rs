use crate::Rgb;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Yuv {
    pub y: u8,
    pub u: u8,
    pub v: u8,
}

#[inline]
pub(crate) fn rgb_to_yuv(pixel: Rgb) -> Yuv {
    let r = i64::from(pixel.r);
    let g = i64::from(pixel.g);
    let b = i64::from(pixel.b);
    // Dayton Appendix B: studio-range Rec.601 Y, R-Y and B-Y.
    let y = 16 + div_round(65_738 * r + 129_057 * g + 25_064 * b, 256_000);
    let u = 128 + div_round(-37_945 * r - 74_494 * g + 112_439 * b, 256_000);
    let v = 128 + div_round(112_439 * r - 94_154 * g - 18_285 * b, 256_000);
    Yuv {
        y: clamp_u8(y),
        u: clamp_u8(u),
        v: clamp_u8(v),
    }
}

#[inline]
pub(crate) fn yuv_to_rgb(yuv: Yuv) -> Rgb {
    let y = i32::from(yuv.y) - 16;
    let u = i32::from(yuv.u) - 128;
    let v = i32::from(yuv.v) - 128;
    Rgb {
        r: clamp_u8((298 * y + 409 * v + 128) >> 8),
        g: clamp_u8((298 * y - 100 * u - 208 * v + 128) >> 8),
        b: clamp_u8((298 * y + 516 * u + 128) >> 8),
    }
}

#[inline]
fn div_round(numerator: i64, denominator: i64) -> i32 {
    if numerator >= 0 {
        ((numerator + denominator / 2) / denominator) as i32
    } else {
        ((numerator - denominator / 2) / denominator) as i32
    }
}

#[inline]
fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rec601_primary_color_vectors_match_dayton_profile() {
        assert_eq!(
            rgb_to_yuv(Rgb::new(0, 0, 0)),
            Yuv {
                y: 16,
                u: 128,
                v: 128
            }
        );
        assert_eq!(
            rgb_to_yuv(Rgb::new(255, 255, 255)),
            Yuv {
                y: 235,
                u: 128,
                v: 128
            }
        );
        assert_eq!(
            rgb_to_yuv(Rgb::new(255, 0, 0)),
            Yuv {
                y: 81,
                u: 90,
                v: 240
            }
        );
        assert_eq!(
            rgb_to_yuv(Rgb::new(0, 255, 0)),
            Yuv {
                y: 145,
                u: 54,
                v: 34
            }
        );
        assert_eq!(
            rgb_to_yuv(Rgb::new(0, 0, 255)),
            Yuv {
                y: 41,
                u: 240,
                v: 110
            }
        );
    }
}

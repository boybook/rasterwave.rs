/// Color channel used by sequential SSTV scan layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Red RGB channel.
    Red,
    /// Green RGB channel.
    Green,
    /// Blue RGB channel.
    Blue,
    /// Luminance channel.
    Luma,
    /// Blue-difference chrominance.
    ChromaBlue,
    /// Red-difference chrominance.
    ChromaRed,
}

/// Pixel-domain representation transmitted by a mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorLayout {
    /// One luminance channel.
    Monochrome,
    /// Sequential RGB components.
    Rgb,
    /// Luminance plus color-difference components.
    Yuv,
}

/// Evidence level attached to a built-in mode profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeStatus {
    /// Primary profile selected from the recorded author, firmware, or
    /// interoperability sources. This is not a conformance certification.
    Canonical,
    /// Historical compatibility profile with conflicting or incomplete source
    /// definitions. Applications should expose the exact profile name.
    Compatibility,
}

/// How one or more image rows are arranged in each radio scan line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScanLayout {
    /// A single grayscale scan follows each sync pulse.
    Monochrome {
        /// Scan duration in seconds.
        scan_seconds: f64,
    },
    /// Martin-style sync-first sequential RGB.
    Martin {
        /// Duration of each color scan.
        channel_seconds: f64,
    },
    /// Scottie-style RGB with the sync pulse between blue and red.
    Scottie {
        /// Duration of each color scan.
        channel_seconds: f64,
    },
    /// Robot color modes.
    Robot {
        /// Duration of the luminance scan.
        luma_seconds: f64,
        /// Duration of each chrominance scan.
        chroma_seconds: f64,
        /// Whether red/blue chroma alternate between rows.
        alternating_chroma: bool,
        /// Color-identification/separator duration.
        separator_seconds: f64,
        /// Porch between color identification and chroma.
        chroma_porch_seconds: f64,
    },
    /// Paul-Davis modes, carrying two luminance rows per radio line.
    Pd {
        /// Duration of each Y/R-Y/B-Y channel.
        channel_seconds: f64,
    },
    /// Wraase SC2 modes with possibly shortened red/blue channels.
    Wraase {
        /// Base channel duration.
        channel_seconds: f64,
        /// Red and blue channel duration relative to green.
        outer_channel_scale: f64,
    },
    /// Pasokon modes with equal sequential RGB channels.
    Pasokon {
        /// Duration of each color scan.
        channel_seconds: f64,
    },
}

/// Supported analog SSTV mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SstvMode {
    /// Robot 8-second black and white.
    Robot8Bw,
    /// Robot 12-second black and white.
    Robot12Bw,
    /// Robot 24-second black and white.
    Robot24Bw,
    /// Robot 36-second black and white.
    Robot36Bw,
    /// Robot 12 color.
    Robot12,
    /// Robot 24 color.
    Robot24,
    /// Robot 36 color.
    Robot36,
    /// Robot 72 color.
    Robot72,
    /// Martin M1.
    Martin1,
    /// Martin M2.
    Martin2,
    /// Martin M3.
    Martin3,
    /// Martin M4.
    Martin4,
    /// Scottie S1.
    Scottie1,
    /// Scottie S2.
    Scottie2,
    /// Scottie S3.
    Scottie3,
    /// Scottie S4.
    Scottie4,
    /// Scottie DX.
    ScottieDx,
    /// PD50.
    Pd50,
    /// PD90.
    Pd90,
    /// PD120.
    Pd120,
    /// PD160.
    Pd160,
    /// PD180.
    Pd180,
    /// PD240.
    Pd240,
    /// PD290.
    Pd290,
    /// Wraase SC2-30.
    WraaseSc2_30,
    /// Wraase SC2-60.
    WraaseSc2_60,
    /// Wraase SC2-120.
    WraaseSc2_120,
    /// Wraase SC2-180.
    WraaseSc2_180,
    /// Pasokon P3.
    Pasokon3,
    /// Pasokon P5.
    Pasokon5,
    /// Pasokon P7.
    Pasokon7,
}

/// Immutable parameters shared by the encoder and decoder for one mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeSpec {
    /// Mode identifier.
    pub mode: SstvMode,
    /// Human-readable mode name.
    pub name: &'static str,
    /// Seven-bit VIS code.
    pub vis_code: u8,
    /// Image width.
    pub width: u32,
    /// Image height.
    pub height: u32,
    /// Pixel-domain representation.
    pub color: ColorLayout,
    /// Scan organization.
    pub layout: ScanLayout,
    /// Sync-pulse duration in seconds.
    pub sync_seconds: f64,
    /// Porch duration following a sync pulse.
    pub porch_seconds: f64,
    /// Nominal duration of one radio scan line.
    pub line_seconds: f64,
    /// Number of image rows emitted by one radio scan line.
    pub rows_per_line: u8,
}

impl ModeSpec {
    /// Evidence level for this exact built-in profile.
    pub const fn status(self) -> ModeStatus {
        match self.mode {
            SstvMode::Robot12
            | SstvMode::WraaseSc2_30
            | SstvMode::WraaseSc2_60
            | SstvMode::WraaseSc2_120 => ModeStatus::Compatibility,
            _ => ModeStatus::Canonical,
        }
    }
}

impl SstvMode {
    /// Return this mode's shared specification.
    pub fn spec(self) -> &'static ModeSpec {
        SSTV_MODES
            .iter()
            .find(|spec| spec.mode == self)
            .expect("every public SstvMode has a specification")
    }

    /// Resolve a supported mode from its seven-bit VIS code.
    pub fn from_vis(vis_code: u8) -> Option<Self> {
        match vis_code & 0x7f {
            1..=3 => return Some(Self::Robot8Bw),
            5..=7 => return Some(Self::Robot12Bw),
            9..=11 => return Some(Self::Robot24Bw),
            13..=15 => return Some(Self::Robot36Bw),
            _ => {}
        }
        SSTV_MODES
            .iter()
            .find(|spec| spec.vis_code == (vis_code & 0x7f))
            .map(|spec| spec.mode)
    }
}

#[allow(clippy::too_many_arguments)]
const fn spec(
    mode: SstvMode,
    name: &'static str,
    vis_code: u8,
    width: u32,
    height: u32,
    color: ColorLayout,
    layout: ScanLayout,
    sync_seconds: f64,
    porch_seconds: f64,
    line_seconds: f64,
    rows_per_line: u8,
) -> ModeSpec {
    ModeSpec {
        mode,
        name,
        vis_code,
        width,
        height,
        color,
        layout,
        sync_seconds,
        porch_seconds,
        line_seconds,
        rows_per_line,
    }
}

const fn martin(
    mode: SstvMode,
    name: &'static str,
    vis: u8,
    height: u32,
    channel: f64,
) -> ModeSpec {
    const SYNC: f64 = 0.004_862;
    const PORCH: f64 = 0.000_572;
    spec(
        mode,
        name,
        vis,
        320,
        height,
        ColorLayout::Rgb,
        ScanLayout::Martin {
            channel_seconds: channel,
        },
        SYNC,
        PORCH,
        SYNC + PORCH + 3.0 * (channel + PORCH),
        1,
    )
}

const fn scottie(
    mode: SstvMode,
    name: &'static str,
    vis: u8,
    height: u32,
    channel: f64,
) -> ModeSpec {
    const SYNC: f64 = 0.009;
    const PORCH: f64 = 0.0015;
    spec(
        mode,
        name,
        vis,
        320,
        height,
        ColorLayout::Rgb,
        ScanLayout::Scottie {
            channel_seconds: channel,
        },
        SYNC,
        PORCH,
        SYNC + 3.0 * (channel + PORCH),
        1,
    )
}

const fn pd(
    mode: SstvMode,
    name: &'static str,
    vis: u8,
    width: u32,
    height: u32,
    channel: f64,
) -> ModeSpec {
    const SYNC: f64 = 0.020;
    const PORCH: f64 = 0.002_080;
    spec(
        mode,
        name,
        vis,
        width,
        height,
        ColorLayout::Yuv,
        ScanLayout::Pd {
            channel_seconds: channel,
        },
        SYNC,
        PORCH,
        SYNC + PORCH + 4.0 * channel,
        2,
    )
}

#[allow(clippy::too_many_arguments)]
const fn robot(
    mode: SstvMode,
    name: &'static str,
    vis: u8,
    width: u32,
    height: u32,
    luma: f64,
    chroma: f64,
    alternating: bool,
    sync: f64,
    porch: f64,
    separator: f64,
    chroma_porch: f64,
) -> ModeSpec {
    let chroma_count = if alternating { 1.0 } else { 2.0 };
    spec(
        mode,
        name,
        vis,
        width,
        height,
        ColorLayout::Yuv,
        ScanLayout::Robot {
            luma_seconds: luma,
            chroma_seconds: chroma,
            alternating_chroma: alternating,
            separator_seconds: separator,
            chroma_porch_seconds: chroma_porch,
        },
        sync,
        porch,
        sync + porch + luma + chroma_count * (separator + chroma_porch + chroma),
        1,
    )
}

const fn bw(
    mode: SstvMode,
    name: &'static str,
    vis: u8,
    width: u32,
    height: u32,
    sync_seconds: f64,
    pixel_seconds: f64,
) -> ModeSpec {
    let scan_seconds = width as f64 * pixel_seconds;
    spec(
        mode,
        name,
        vis,
        width,
        height,
        ColorLayout::Monochrome,
        ScanLayout::Monochrome { scan_seconds },
        sync_seconds,
        0.0,
        sync_seconds + scan_seconds,
        1,
    )
}

/// Complete built-in SSTV mode catalog.
pub static SSTV_MODES: &[ModeSpec] = &[
    bw(
        SstvMode::Robot8Bw,
        "Robot 8 BW",
        2,
        160,
        120,
        0.010,
        0.000_350,
    ),
    bw(
        SstvMode::Robot12Bw,
        "Robot 12 BW",
        6,
        160,
        120,
        0.007,
        0.000_581_25,
    ),
    bw(
        SstvMode::Robot24Bw,
        "Robot 24 BW",
        10,
        320,
        240,
        0.012,
        0.000_275,
    ),
    bw(
        SstvMode::Robot36Bw,
        "Robot 36 BW",
        14,
        320,
        240,
        0.012,
        0.000_431_25,
    ),
    robot(
        SstvMode::Robot12,
        "Robot 12",
        0,
        160,
        120,
        0.060,
        0.030,
        true,
        0.007,
        0.0,
        0.003,
        0.0,
    ),
    robot(
        SstvMode::Robot24,
        "Robot 24",
        4,
        160,
        120,
        0.088,
        0.044,
        false,
        0.009,
        0.003,
        0.0045,
        0.0015,
    ),
    robot(
        SstvMode::Robot36,
        "Robot 36",
        8,
        320,
        240,
        0.088,
        0.044,
        true,
        0.009,
        0.003,
        0.0045,
        0.0015,
    ),
    robot(
        SstvMode::Robot72,
        "Robot 72",
        12,
        320,
        240,
        0.138,
        0.069,
        false,
        0.009,
        0.003,
        0.0045,
        0.0015,
    ),
    martin(SstvMode::Martin1, "Martin M1", 44, 256, 0.146_432),
    martin(SstvMode::Martin2, "Martin M2", 40, 256, 0.073_216),
    martin(SstvMode::Martin3, "Martin M3", 36, 128, 0.146_432),
    martin(SstvMode::Martin4, "Martin M4", 32, 128, 0.073_216),
    scottie(SstvMode::Scottie1, "Scottie S1", 60, 256, 0.138_240),
    scottie(SstvMode::Scottie2, "Scottie S2", 56, 256, 0.088_064),
    scottie(SstvMode::Scottie3, "Scottie S3", 52, 128, 0.138_240),
    scottie(SstvMode::Scottie4, "Scottie S4", 48, 128, 0.088_064),
    scottie(SstvMode::ScottieDx, "Scottie DX", 76, 256, 0.345_600),
    pd(SstvMode::Pd50, "PD50", 93, 320, 256, 0.091_520),
    pd(SstvMode::Pd90, "PD90", 99, 320, 256, 0.170_240),
    pd(SstvMode::Pd120, "PD120", 95, 640, 496, 0.121_600),
    pd(SstvMode::Pd160, "PD160", 98, 512, 400, 0.195_584),
    pd(SstvMode::Pd180, "PD180", 96, 640, 496, 0.183_040),
    pd(SstvMode::Pd240, "PD240", 97, 640, 496, 0.244_480),
    pd(SstvMode::Pd290, "PD290", 94, 800, 616, 0.228_800),
    spec(
        SstvMode::WraaseSc2_30,
        "Wraase SC2-30",
        51,
        320,
        128,
        ColorLayout::Rgb,
        ScanLayout::Wraase {
            channel_seconds: 0.117,
            outer_channel_scale: 0.5,
        },
        0.005,
        0.0,
        0.239,
        1,
    ),
    spec(
        SstvMode::WraaseSc2_60,
        "Wraase SC2-60",
        59,
        320,
        256,
        ColorLayout::Rgb,
        ScanLayout::Wraase {
            channel_seconds: 0.117,
            outer_channel_scale: 0.5,
        },
        0.005,
        0.0,
        0.239,
        1,
    ),
    spec(
        SstvMode::WraaseSc2_120,
        "Wraase SC2-120",
        63,
        320,
        256,
        ColorLayout::Rgb,
        ScanLayout::Wraase {
            channel_seconds: 0.235,
            outer_channel_scale: 0.5,
        },
        0.005,
        0.0,
        0.475,
        1,
    ),
    spec(
        SstvMode::WraaseSc2_180,
        "Wraase SC2-180",
        55,
        320,
        256,
        ColorLayout::Rgb,
        ScanLayout::Wraase {
            channel_seconds: 0.235,
            outer_channel_scale: 1.0,
        },
        0.005_522_5,
        0.0005,
        0.711_022_5,
        1,
    ),
    spec(
        SstvMode::Pasokon3,
        "Pasokon P3",
        113,
        640,
        496,
        ColorLayout::Rgb,
        ScanLayout::Pasokon {
            channel_seconds: 0.133_333_333,
        },
        0.005_208_333,
        0.001_041_667,
        0.409_375,
        1,
    ),
    spec(
        SstvMode::Pasokon5,
        "Pasokon P5",
        114,
        640,
        496,
        ColorLayout::Rgb,
        ScanLayout::Pasokon {
            channel_seconds: 0.200,
        },
        0.007_812_5,
        0.001_562_5,
        0.614_062_5,
        1,
    ),
    spec(
        SstvMode::Pasokon7,
        "Pasokon P7",
        115,
        640,
        496,
        ColorLayout::Rgb,
        ScanLayout::Pasokon {
            channel_seconds: 0.266_666_667,
        },
        0.010_416_667,
        0.002_083_333,
        0.818_75,
        1,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_timing_examples_match_sources() {
        assert!((SstvMode::Robot24.spec().line_seconds - 0.200).abs() < 1e-12);
        assert!((SstvMode::Pd120.spec().line_seconds - 0.508_480).abs() < 1e-12);
        assert!((SstvMode::Pd160.spec().line_seconds - 0.804_416).abs() < 1e-12);
        assert!((SstvMode::Pasokon3.spec().line_seconds - 0.409_375).abs() < 1e-12);
        assert_eq!(SstvMode::Pasokon3.spec().width, 640);
        assert_eq!(SstvMode::Pasokon3.spec().height, 496);
    }

    #[test]
    fn bw_vis_variants_map_to_one_wire_geometry() {
        for code in 1..=3 {
            assert_eq!(SstvMode::from_vis(code), Some(SstvMode::Robot8Bw));
        }
        for code in 5..=7 {
            assert_eq!(SstvMode::from_vis(code), Some(SstvMode::Robot12Bw));
        }
    }
}

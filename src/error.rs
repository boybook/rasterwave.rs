use crate::SstvMode;

/// Result type used by Rasterwave APIs.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by codec construction and validation.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A sample rate cannot represent the required SSTV or radiofax tones.
    #[error("sample rate {0} Hz is outside the supported range 8,000..=384,000 Hz")]
    InvalidSampleRate(u32),

    /// An image buffer length does not match its declared dimensions.
    #[error("image buffer has {actual} pixels, expected {expected}")]
    InvalidImageBuffer {
        /// Expected number of pixels.
        expected: usize,
        /// Actual number of pixels.
        actual: usize,
    },

    /// An interleaved RGB byte buffer does not contain exactly three bytes per
    /// declared pixel.
    #[error("RGB byte buffer has {actual} bytes, expected {expected}")]
    InvalidRgbByteBuffer {
        /// Expected byte count.
        expected: usize,
        /// Actual byte count.
        actual: usize,
    },

    /// The input image does not have the dimensions required by a mode.
    #[error(
        "{mode:?} requires {expected_width}x{expected_height}, got {actual_width}x{actual_height}"
    )]
    ImageDimensions {
        /// Requested SSTV mode.
        mode: SstvMode,
        /// Required width.
        expected_width: u32,
        /// Required height.
        expected_height: u32,
        /// Supplied width.
        actual_width: u32,
        /// Supplied height.
        actual_height: u32,
    },

    /// A numeric configuration value is invalid.
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(&'static str),

    /// A PCM chunk contained NaN or infinity.
    #[error("PCM input contains a non-finite sample")]
    NonFiniteSample,

    /// Samples were supplied after the decoder was finalized.
    #[error("decoder has already been finished; call reset before supplying more input")]
    DecoderFinished,
}

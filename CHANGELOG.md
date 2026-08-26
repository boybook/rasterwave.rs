# Changelog

## Unreleased

## 0.3.1 - 2026-08-26

- Jointly search radiofax dead-sector phase and clock ppm on a circular 64-line
  window, including 1 ppm refinement for small full-page slant.
- Require multiple time-consistent image windows before publishing a
  calibration point, preventing isolated map features from moving a segment.
- Use trusted phasing as a horizontal prior while allowing the image tracker
  to correct residual clock error throughout a transmission.
- Reject structureless input before the expensive search and narrow later
  searches around the established clock to keep continuous receive bounded.

## 0.3.0 - 2026-08-25

- Recover radiofax line period and horizontal phase with robust phasing
  statistics instead of freezing a short ordinary least-squares window.
- Project framed page starts onto the recovered line grid and invalidate stale
  clock evidence after rejected phasing cycles.
- Add nominal-paper calibration events, dead-sector tracking, and a bounded
  cross-row paper correction helper for continuous receivers.
- Expose clock source, status, confidence, phase, ppm, and raster-basis
  metadata while preserving immediate paper output.

## 0.2.0 - 2026-08-24

- Add first-class `SstvPaperDecoder` and `FaxPaperDecoder` orchestration APIs.
- Print fallback raster rows immediately while protocol acquisition runs in
  parallel, with monotonic paper rows and explicit trusted boundaries.
- Complete SSTV captures only after a trusted start plus nominal image height,
  and fax captures only after confirmed APT stop; paper output continues.
- Add automatic fax IOC/LPM and FM/AM acquisition paths, manual mismatch
  reporting, discontinuity dividers, and encode/decode integration coverage.

## 0.1.1 - 2026-08-24

- Add opt-in immediate decoding for every SSTV mode when `manual_mode` is set.
- Add opt-in immediate radiofax decoding when IOC and LPM are fixed.
- Keep emitting rows through low signal levels while using later SSTV sync
  pulses for clock, phase, and frequency correction.

## 0.1.0 - 2026-08-24

- Initial streaming SSTV encoder and decoder.
- Automatic VIS candidate detection and sync-timing confirmation.
- No-VIS candidate inference with ambiguity preservation.
- Online frequency offset and robust line-clock tracking.
- Streaming WMO-style radiofax encode/decode API, including IOC 288/576,
  standard phasing/APT framing, and parameterized FM and AM subcarriers.
- Thread-safety, timing, chunking, radiofax, and Criterion coverage.

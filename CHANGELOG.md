# Changelog

## Unreleased

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

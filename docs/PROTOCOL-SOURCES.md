# Protocol Sources And Decisions

## Evidence Order

Rasterwave resolves conflicting tables in this order:

1. current WMO/ITU normative material, where applicable;
2. a mode author's proposal or original firmware behavior;
3. multiple independent interoperable implementations;
4. handbooks and secondary summaries.

An implementation majority does not override a primary mathematical or wire
definition. Disputed profiles are named and marked as compatibility profiles.
`ModeStatus::Canonical` is the API name for the project's preferred reading of
a profile. It is not a standards-body designation, conformance certificate, or
claim that every third-party implementation uses identical timing.

## SSTV Primary Sources

- J. L. Barber, N7CXI, *Proposal for SSTV Mode Specifications*, Dayton 2000:
  <https://web.archive.org/web/20190828130738id_/http://www.barberdsp.com:80/downloads/Dayton%20Paper.pdf>
- Mirror used to verify the archived file:
  <https://raw.githubusercontent.com/dfdias/SSTV/master/Dayton%20Paper.pdf>
- Verified PDF SHA-256:
  `b3747964d8b3599c7f2236717f3e6519b57a5dd86942b7f509478efcbf616d49`
- Martin Bruchanov, OK2MNM, *Image Communication on Short Waves*:
  <https://www.sstv-handbook.com/>
- QSSTV transmission sequencing and DL3YAP FSK-ID interoperability reference:
  <https://github.com/ON4QZ/QSSTV/blob/main/src/sstv/sstvtx.cpp>
- Robot36 companion encoder reference for the baseline 910 ms VIS header:
  <https://github.com/xdsopl/SSTVEncoder2/blob/master/app/src/main/java/om/sstvencoder/Modes/Mode.java>

The Dayton document is a proposal, not a formal standards-body publication.
It remains the strongest first-party engineering source for Robot 1200C
Scottie/Martin/Robot/Wraase behavior and author-supplied Pasokon/PD/FAX data.

## SSTV Mathematical Model

SSTV is a time-ordered piecewise-FM raster grammar:

```text
f(v) = 1500 Hz + (800 Hz / 255) * v
pixel_time = component_scan_time / source_columns
phase(t) = phase(0) + 2*pi*integral(f(t) dt)
```

VIS is 910 ms total:

```text
1900/300 ms, 1200/10 ms, 1900/300 ms,
start 1200/30 ms,
7 data bits LSB-first (1=1100, 0=1300),
even parity/30 ms, stop 1200/30 ms
```

Rasterwave stores the seven-bit payload. The encoder adds parity at the wire
boundary.

Robot and PD color use the studio-range Rec.601 transform recorded in the
Dayton appendix, including the 16/235 luminance and centered color-difference
ranges:

```text
Y   = 16  + ( 65.738 R + 129.057 G +  25.064 B) / 256
B-Y = 128 + (-37.945 R -  74.494 G + 112.439 B) / 256
R-Y = 128 + (112.439 R -  94.154 G -  18.285 B) / 256
```

Values are rounded and clamped to an eight-bit wire value.

## SSTV Family Provenance Matrix

This matrix records the source revision and the implementation decision in
crate version 0.1.0. "Primary" below maps to the public
`ModeStatus::Canonical` value with the limited meaning stated above.

| Family | Primary provenance / revision | Built-in scope | 0.1.0 decision |
| --- | --- | --- | --- |
| Robot BW | Dayton proposal, 2000; OK2MNM handbook cross-check | BW 8/12/24/36 | Primary profiles; legacy VIS aliases resolve to the matching BW family. |
| Robot color | Dayton proposal, 2000, including Appendix B | Color 12/24/36/72 | 24/36/72 are primary readings; Color 12 is compatibility due to conflicting timing. |
| Martin | Dayton proposal, 2000; OK2MNM handbook cross-check | M1/M2/M3/M4 | Primary source-backed profiles, not a conformance certification. |
| Scottie | Dayton proposal, 2000; OK2MNM handbook cross-check | S1/S2/S3/S4/DX | Primary profiles include the Robot 1200C first-sync prefix. Timing-identical S1/S3 and S2/S4 remain ambiguous without VIS; unique DX timing can lock automatically, and manual mode can resolve an ambiguous candidate set from buffered history. |
| PD | Dayton proposal, 2000, author-supplied tables | PD50/90/120/160/180/240/290 | Primary profiles use studio-range Rec.601 color and the corrected PD160 timing below. |
| Pasokon | Dayton proposal, 2000, author-supplied tables | P3/P5/P7 | Primary profiles use the 640x496 wire raster. |
| Wraase SC2 | Dayton proposal, 2000; OK2MNM handbook cross-check | SC2-30/60/120/180 | SC2-180 is the primary reading; 30/60/120 are explicit compatibility profiles. |

## Recorded SSTV Decisions

- PD160 scan time is 195.584 ms. 195.854 ms is inconsistent with
  `512 * 382 us`.
- Pasokon P3/P5/P7 use 640 source columns. Smaller handbook values describe
  effective resolution, not the wire raster.
- Robot Color 24 uses 120 radio rows and 200 ms per row. The default logical
  profile is 160x120; a future oversampled 320-column profile must be named
  separately.
- Scottie primary profiles have the Robot 1200C first-sync prefix.
- Wraase SC2-30/60/120 and Robot Color 12 remain compatibility profiles due to
  conflicting historical definitions.
- FAX480 is not represented as an ordinary VIS SSTV mode. Original framing is
  APT plus phasing; later VIS 85 is a distinct compatibility profile that is
  not implemented yet.

## Meteorological Radiofax Sources

- WMO-No. 386, 2023 edition, Part III section 5:
  <https://amc.namem.gov.mn/wp-content/uploads/WMO/5.%20386_2023-edition_en.pdf>
- CCIR Recommendation 343-1:
  <https://search.itu.int/history/HistoryDigitalCollectionDocLibrary/4.283.43.en.1005.pdf>
- ISO 9876:2015 receiver standard:
  <https://www.iso.org/standard/66059.html>
- NOAA worldwide marine radiofacsimile schedule (2025-03-07):
  <https://www.weather.gov/media/marine/rfax.pdf>

## Radiofax Geometry

WMO defines the index of cooperation geometrically:

```text
IOC M = L * F / pi
T_line = 60 / LPM seconds
```

`L` is full scan-line length and `F` is scanning density. IOC is not a pixel
count. With square sampling, Rasterwave uses these explicit full and active
widths:

```text
IOC                         288    576
full raster samples         905   1810
active picture samples      864   1728
```

The full raster includes the 4.5% phasing/dead sector. `FaxEncoder` accepts a
`GrayImage` at either the active width (recommended) or the full width. In both
cases it generates the reserved sector itself; pixels supplied beyond the
active width in a full-raster input are replaced by the white phasing level.
`FaxDecoder::LineReady` always returns a full-width row, so consumers that need
only the picture crop it to `FaxIoc::active_width()`.

WMO FM maps black=1500 Hz and white=2300 Hz around a 1900 Hz center. IOC
selection alternates black/white levels with a 300 or 675 Hz rectangular
envelope for 5-10 seconds; those are not direct audio tones. Phasing lasts
about 30 seconds with either symmetrical black/white phasing or a 5% white
pulse on black. Stop alternates at 450 Hz for five seconds and is followed by
ten seconds of black.

Radiofax height is intentionally open-ended.

## Radiofax Rate And Acquisition Scope

WMO-No. 386 lists 60, 90, 120, and 240 lines per minute. Those four values are
represented by `FaxLpm` constants, return `true` from `FaxLpm::is_wmo()`, and
are the rates considered by automatic phasing inference. `FaxLpm::LPM_180` is
an interoperability extension, not a WMO-listed rate; configure it explicitly
on the decoder. `FaxLpm::new` accepts other 30-480 LPM extensions, which also
require explicit receive configuration.

The implemented receive paths are deliberately bounded:

- Default WMO-style acquisition confirms the APT envelope, detects IOC 288 or
  576, then locks line rate and horizontal phase from phasing.
- Phasing-only acquisition is supported when the caller preconfigures IOC.
  Preconfigure LPM as well for 180 LPM or another extension.
- A stream with neither APT nor phasing cannot be acquired automatically.
- A confirmed 450 Hz stop envelope closes the page without emitting the held
  stop samples as image data. `max_lines` is an independent caller bound.
- EOF and configured target-carrier level/coherence loss close an active page
  as partial. Silence and wideband noise do not free-run the raster clock into
  synthetic rows; receiver-specific integrations may apply an upstream squelch.

`FaxModulation::WMO_FM` is the WMO 1900 Hz +/-400 Hz audio subcarrier.
Parameterized FM (including inverted polarity) and parameterized AM are both
implemented for encode and decode, with clean PCM self-loop coverage. That
coverage establishes internal codec consistency; it is not by itself proof of
over-air or third-party receiver interoperability. Encoder and decoder must be
configured with the same non-default modulation parameters.

## Golden-Test Policy

Golden data should prefer event timelines and instantaneous frequencies over
opaque WAV byte fixtures. Tests should cover:

- VIS bits/parity and total duration;
- first-row exceptions and alternating row programs;
- source-column counts and row-pair layouts;
- complete timeline error at 8/44.1/48/96 kHz;
- frequency offset, clock ppm, chunking, discontinuity, clipping and noise;
- external decode by implementations whose licenses permit use as an oracle.

Real-radio corpora may be used locally only when redistribution rights are
unclear.

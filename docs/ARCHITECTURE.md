# Architecture

## Design Goal

Rasterwave models analog image protocols as bounded, ordered state machines.
It must accept arbitrary PCM chunk sizes, emit useful rows before EOF, and
produce the same samples/events when driven one sample at a time or in one
large call.

## SSTV Receive Path

```text
caller PCM
  -> streaming linear rate conversion (12 kHz working rate)
  -> quadrature downmix around 1900 Hz
  -> two-stage low-pass + complex phase-difference FM demodulation
  -> parallel VIS header and sync-pulse detectors
  -> candidate/confirmation policy
  -> robust line-clock regression
  -> family row decoder
  -> borrowed LineReady events
```

The acquisition frequency ring is fixed at eight seconds. It allows a VIS
candidate to be confirmed by later sync pulses without discarding the early
rows. Once locked, the decoder reuses fixed line and channel scratch buffers.

VIS parity alone is not enough under low SNR: a corrupted word can still have
valid even parity and name another supported mode. Rasterwave reports the VIS
candidate first, then requires compatible sync width and repeated line period
before emitting `ImageStarted`.

No-VIS inference is stricter. Six compatible pulses are scored against the
mode catalog. If line timing cannot distinguish modes such as Martin M1/M3 or
Scottie S1/S3, only `ModeCandidate` is emitted. A caller may then lock a manual
mode; the library does not choose arbitrarily.

## Frequency And Clock Tracking

Frequency offset is estimated relative to the 1900 Hz leader and refined from
1200 Hz sync pulses. All pixel frequencies use the same estimate.

Clock drift is distinct from carrier offset. Compatible sync edges receive a
line index that can skip missed pulses. A least-squares fit over the latest
eight `(line_index, sample_position)` points updates samples-per-line and
re-anchors the next line boundary from the fitted phase only when:

- slope is within +/-2000 ppm of the profile;
- at least four points exist;
- residual RMS is below the sync-jitter threshold.

Line boundaries use a cumulative fractional deadline. They never round every
row independently.

## Row Revisions

Robot 36 alternates red-difference and blue-difference chroma. An even row can
be displayed immediately with neutral missing chroma (`Provisional`). The next
radio row then produces a revised final even row and a final odd row. Consumers
must key rows by `(image_id, line_index, revision)`.

## Transmit Path

The encoder stores the source raster, mode state, oscillator recurrence, and a
single cumulative sample deadline. It creates tone or pixel segments lazily and
fills caller memory directly. The oscillator is phase-continuous across pixels,
segments, and caller chunk boundaries.

## Radiofax Boundary

Radiofax shares PCM, oscillator, demodulation, and raster primitives with SSTV
but has a separate protocol state machine:

```text
APT selection -------------------> phasing/IOC/LPM lock -> image rows
configured IOC + phasing-only ---> phasing/IOC/LPM lock -> image rows
image rows -> confirmed APT stop | max-lines | EOF | signal-loss
```

IOC is not a fixed image size. The built-in square-sampling policy derives a
full line width, while page height remains open until stop or a configured
limit. The encoder accepts either active-picture or full-raster input; the
decoder emits full-raster rows. APT and suspected stop samples are held until
the control pattern is confirmed so they are not exposed as image pixels.

The default acquisition path detects IOC from APT and WMO line rates from
phasing. Phasing-only acquisition requires the IOC in `FaxDecoderConfig`; an
LPM outside the automatically inferred WMO set must also be configured. An
image-only stream has no timing or horizontal-phase acquisition marker and is
not acquired automatically.

Parameterized FM and AM subcarriers share the framing state machine. Receive
configuration must name the modulation expected on the input. Low target-carrier
level or coherence for the configured interval ends an active page as partial
instead of allowing silence or wideband noise to synthesize more rows. A caller
with receiver-specific squelch evidence may set the coherence threshold to zero
and end the page explicitly at its integration boundary.

## Threading And Future Node.js Binding

The core exposes synchronous methods taking `&mut self`. This is the correct
primitive for a future Promise API:

```text
JS handle -> per-handle serial work queue -> owned Rust codec session
                                            -> owned events back to JS
```

Different handles can run on a Node worker pool. Work for one handle must stay
ordered. The core does not embed Tokio, a thread pool, an `Arc<Mutex<_>>`, or a
Node-API dependency.

Borrowed events are the zero-copy Rust path. A binding must copy
`DecodeEventRef` with `to_owned()` before crossing an asynchronous boundary.

## Memory Invariants

- no mutable process-global codec state;
- acquisition history has fixed capacity;
- locked SSTV audio buffers retain only pending rows;
- radiofax retains bounded phasing history plus bounded stop/fade evidence;
- owned complete images are created only when the caller asks to retain them;
- no `unsafe` code in the core crate.

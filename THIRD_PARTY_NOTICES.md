# Third-Party Notices

Rasterwave is an independent Rust implementation. Its protocol research and
architecture were informed by the following open-source projects. No GPL UI or
application source is vendored or linked into this crate.

## slowrx

- Project: <https://github.com/windytan/slowrx>
- Author: Oona Raisanen (OH2EIQ)
- License: ISC
- Used for: SSTV synchronization, VIS, frequency-offset, and historical mode
  behavior research.

## slowrx.rs

- Project: <https://github.com/jasonherald/slowrx.rs>
- Author: Jason Herald and contributors
- License: MIT, with upstream slowrx ISC attribution
- Used for: Rust API comparison, real-capture validation notes, and review of
  two-pass slant correction tradeoffs.

## Robot36

- Project: <https://github.com/xdsopl/robot36>
- Author: Ahmet Inan and contributors
- License: 0BSD
- Used for: online sync-period mode candidates, bounded buffering, frequency
  offset tracking, and adjacent-line chroma behavior research.

## libsstv

- Project: <https://github.com/rimio/libsstv>
- Author: Vasile Vilvoiu (YO7JBP)
- License: MIT
- Used for: phase-continuous incremental encoder state-machine research and
  mode-table cross-checking. Rasterwave does not use libsstv's process-global
  allocator or static encoder contexts.

## CSDR SSTV And Fax Modules

- Project: <https://github.com/luarvique/csdr>
- Relevant files: `include/sstv.hpp`, `src/lib/sstv.cpp`, `include/fax.hpp`,
  `src/lib/fax.cpp`
- Author: Marat Fayzullin and contributors
- File license: BSD-3-Clause (the repository contains mixed licensing)
- Used for: streaming row protocol and radiofax APT/phasing behavior research.
  Rasterwave does not depend on FFTW or CSDR.

## Other Interoperability References

QSSTV, fldigi, OpenWebRX+, and Isobar were used only as external behavioral or
interoperability references. Their GPL/AGPL code is not copied, linked, or
distributed by Rasterwave.

# Building Rasterwave

## Prerequisites

- Rust 1.85 or newer
- Cargo
- A platform linker/toolchain supported by the selected Rust target

Install Rust with rustup:

```bash
rustup toolchain install 1.85.0 --component rustfmt --component clippy
rustup override set 1.85.0
```

## Development Build

```bash
git clone https://github.com/boybook/rasterwave.rs.git
cd rasterwave.rs
cargo build
cargo test --all-targets
```

The crate is pure safe Rust and has no C library, platform audio, FFTW, GUI,
async-runtime, or Node.js dependency. It does not compile any C or C++ source,
but `cargo build`, tests, examples, and benchmarks still invoke the platform
linker. Install the usual target prerequisites, such as Xcode Command Line
Tools on macOS, MSVC Build Tools for an MSVC Windows target, or a supported
linker on Linux. The same Cargo commands are expected to work on Linux, macOS,
and Windows.

## Release Build

```bash
cargo build --release
```

The repository enables thin LTO and one codegen unit for release/benchmark
profiles. Applications that need faster incremental release builds may
override those profile settings in their workspace root.

## Quality Gates

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo bench --bench codec --no-run
cargo package --list
cargo package
```

Run benchmarks with:

```bash
cargo bench --bench codec
```

## Minimum Rust Version

`Cargo.toml` declares `rust-version = "1.85"`. CI runs the test suite on that
toolchain across Linux, macOS, and Windows. Do not use a newer standard-library
API without either raising the documented MSRV or providing a compatible
implementation.

## Embedding

Add the crate to a Rust workspace normally. A future Node-API wrapper should be
a separate crate so the codec remains usable by desktop, server, mobile, WASM,
and other FFI consumers without Node-specific dependencies.

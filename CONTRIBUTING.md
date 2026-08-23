# Contributing

Contributions are welcome, especially independently captured interoperability
fixtures with clear redistribution rights.

Before changing a mode:

1. identify the primary wire definition;
2. record conflicting sources in `docs/PROTOCOL-SOURCES.md`;
3. keep encoder and decoder parameters in the shared catalog;
4. add a timing or event golden test;
5. test multiple PCM chunk sizes;
6. preserve the no-global-state and bounded-memory invariants.

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo bench --bench codec --no-run
cargo doc --no-deps
```

New core code must remain safe Rust. Discuss any proposed `unsafe`/SIMD module
before implementation; it must be optional and bit-compared with the scalar
path.

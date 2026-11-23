# fntidy
A Rust CLI for Windows 11 that checks and fixes file and folder names for cross-platform and cloud compatibility, with previews, collision detection, and undo.

## Development

With a stable Rust toolchain installed, build and run the current CLI:

```sh
cargo build --locked
cargo run --locked -- --help
cargo run --locked -- --version
cargo test --locked
```

The initial CLI provides a help and version entry point. The first feature
milestone is a read-only `check` for the Windows naming profile. The intended
test boundaries are name rules, rename planning, Windows filesystem operations,
and CLI output.

[Detox](https://github.com/dharple/detox) is a reference for filename cleanup.

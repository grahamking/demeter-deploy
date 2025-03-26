# Demeter Development Guide

## Build Commands
- Full build: `./build.sh`
- Seed build: `cd seed && source env_build.sh && cargo build --release`
- De build: `cd de && cargo build --release`
- Test seed: `cd seed && cargo test`

## Style Guidelines
- **Formatting**: Standard Rust formatting with `cargo fmt`
- **Naming**: Snake_case for variables/functions, CamelCase for types
- **Imports**: Group standard library, external crates, then local modules
- **Error Handling**: Use `anyhow` for error propagation
- **Performance**: Optimize for small binary size with `opt-level = "z"` in release profiles
- **Dependencies**: Keep minimal, currently `anyhow`, `clap`, `crossbeam-channel`, `ssh2`

## Architecture Notes
- `de/`: Client program for synchronization
  - Uses the `Remote` trait for abstraction
  - Worker threads with channels for concurrency
- `seed/`: Small remote helper for checksumming files
  - No-std environment with custom system call implementations
  - Must be built with special flags (use `env_build.sh`)
  - AVX2 instructions used for optimization

## Special Considerations
- Run seed with trailing slashes on directory names
- The seed component must be linked with `-nostartfiles` and other special flags
- The project emphasizes minimalist, efficient code
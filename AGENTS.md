# Repository Guidelines

## Project Structure & Module Organization
- `src/`: safe Rust wrapper crate (`faster-rs` on crates.io, imported as `faster_rs`).
- `libfaster-sys/`: FFI “-sys” crate (CMake build + bindgen) and the vendored FASTER sources at `libfaster-sys/FASTER/`.
- `tests/`: integration tests (`*_tests.rs`).
- `examples/`: runnable examples (`basic.rs`, `custom_keys.rs`, …).
- `benchmark/`: separate benchmarking crate (CLI built with `clap`).
- CI is defined in `azure-pipelines.yml` (Linux build + tests).

## Build, Test, and Development Commands
- `cargo build`: build the Rust wrapper and `libfaster-sys`.
- `cargo build --examples`: ensure examples compile.
- `cargo test`: run unit + integration tests.
- `cargo run --example basic`: run a single example (see `examples/`).
- `cd benchmark && cargo run --release -- help`: benchmark CLI help (`process-ycsb` and `run` subcommands).

System deps (Linux): a C++11 toolchain + CMake + FASTER libs. For Ubuntu, CI uses:
```bash
sudo apt install -y g++-7 libaio-dev uuid-dev libtbb-dev
```

## Coding Style & Naming Conventions
- Rust 2018 edition; keep code `rustfmt`-clean and clippy-friendly (`cargo fmt`, `cargo clippy --all-targets`).
- Keep unsafe/FFI work in `libfaster-sys/`; keep `src/` focused on safe APIs and ergonomics.
- Use idiomatic naming: `UpperCamelCase` types, `snake_case` modules/functions.

## Testing Guidelines
- Prefer adding regression tests under `tests/` for externally visible behavior.
- Use deterministic tests; prefer `tempfile` for filesystem-backed cases.

## Commit & Pull Request Guidelines
- We use `jj` locally; set messages with `jj describe` and keep commits logically scoped.
- Commit subjects follow an imperative style and often use a scope prefix, e.g. `libfaster-sys: …`, `benchmark: …`, `azure-pipelines: …`, `recover: …`.
- PRs should target `master`, describe *what/why*, and list verification (e.g. `cargo test`, `cargo build --examples`). Call out any changes affecting the `libfaster-sys/FASTER/` submodule or system dependencies.

## Release Workflow (FASTER submodule)
- It is OK to use `git` (instead of `jj`) for submodule release workflows.
- Local upstream FASTER repo is at `../FASTER` (sibling of this repo).
- Submodule path used by this repo is `libfaster-sys/FASTER/`.
- Flow: push shim changes in `../FASTER` → update submodule pointer in this repo → update downstream consumers.

## Agent-Specific Notes
- Keep patches focused; add tests for new behavior.
- Don’t add standalone `.rs` files outside Cargo targets—extend existing crates/tests instead.
- Manage dependencies via `cargo add` / `cargo rm` rather than editing `Cargo.toml` by hand.
- During repo inspections, spawn subagents to scan major modules in parallel and consolidate findings with file paths; wait for all subagents to finish before reporting.

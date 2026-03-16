# Agent Instructions

Build/test commands and project learnings. Keep brief. No status updates.

## Project Info

- **Name**: pi_agent_rust (binary: `pi`)
- **Language**: Rust 2024 edition, MSRV 1.85, nightly toolchain
- **Platform**: Linux, macOS, Windows
- **Async runtime**: asupersync (NOT tokio)
- **TUI**: charmed-bubbletea
- **Test**: built-in #[test] + criterion + proptest + insta + loom
- **Package manager**: cargo

## Commands

| Action | Command |
|--------|---------|
| Build (dev) | `cargo build` |
| Build (prod) | `cargo build --release` |
| Build (perf profile) | `cargo build --profile perf` |
| Build (with feature) | `cargo build --features wasm-host` |
| Test all | `cargo test --all-targets` |
| Test one | `cargo test {name} -- --exact` |
| Test pattern | `cargo test {pattern}` |
| Test file | `cargo test --test {integration_test_name}` |
| Test verbose | `cargo test -- --nocapture` |
| Test WASM (Linux) | `cargo test --all-targets --features wasm-host` |
| Lint | `cargo clippy --all-targets -- -D warnings` |
| Lint fix | `cargo clippy --fix --allow-dirty` |
| Format | `cargo fmt` |
| Format check | `cargo fmt --check` |
| Doc check | `cargo doc --no-deps` |
| Clean | `cargo clean` |
| Fuzz | `cargo fuzz run {target} -- -max_total_time=60` |
| Bench | `cargo bench --bench {name}` |

Benchmarks: tools, extensions, system, tui_perf.

## Validation Chain (fast feedback order)

1. `cargo fmt --check` — < 1s
2. `cargo clippy --all-targets -- -D warnings` — 10-30s
3. `cargo doc --no-deps` — 30-60s
4. `cargo build` — 30-120s
5. `cargo test {specific}` — 5-30s
6. `cargo test --all-targets` — 1-10m

## Environment Variables

- `VCR_MODE=playback` and `VCR_CASSETTE_DIR=tests/fixtures/vcr` — required for VCR tests
- `RUST_BACKTRACE=1` — backtraces
- `RUST_LOG=debug` — tracing output
- `CARGO_INCREMENTAL=0` — CI builds
- `RUSTFLAGS="-D warnings"` — CI lint enforcement
- `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY` — for live provider tests

## Key Files

- `src/main.rs` — CLI entry + async orchestrator
- `src/lib.rs` — Public SDK surface
- `src/agent.rs` — Core agent loop
- `src/provider.rs` — Provider trait
- `src/tools.rs` — Built-in tools
- `src/session.rs` — JSONL session persistence
- `src/extensions.rs` — Extension lifecycle
- `src/extension_dispatcher.rs` — Hostcall dispatch
- `src/config.rs` — Config loading
- `src/auth.rs` — OAuth + API keys
- `src/rpc.rs` — RPC JSON protocol
- `src/error.rs` — Error enum
- `src/model.rs` — Message types
- `src/models.rs` — Model registry
- `src/providers/` — 10 LLM provider implementations
- `src/http/` — HTTP client + SSE streaming
- `src/connectors/` — Capability-gated host access
- `src/interactive/` — TUI modules (16 files)
- `src/bin/` — 20 specialized binaries
- `tests/` — 120+ integration tests + fixtures
- `tests/fixtures/vcr/` — VCR cassettes
- `tests/suite_classification.toml` — test categories and quarantine
- `benches/` — Criterion benchmarks (4 targets)
- `fuzz/` — 13 fuzz targets
- `Cargo.toml` — dependencies and features
- `rust-toolchain.toml` — nightly toolchain spec
- `.github/workflows/ci.yml` — main CI pipeline

## Key Patterns

- `#![forbid(unsafe_code)]` — no unsafe anywhere
- Error handling: thiserror enum + `Result<T>` alias + `?` propagation
- Async: asupersync primitives only (Mutex, mpsc, oneshot, timeout, fs)
- Arc wrapping: `Arc<AssistantMessage>`, `Arc<ToolResultMessage>` for cheap clones
- Cow in Context: Provider context borrows from agent state
- Serde: `#[serde(default)]`, `#[serde(rename_all = "snake_case")]`, camelCase aliases for TS compat
- Builder pattern: `#[must_use]` on chainable methods
- Const fn for compile-time defaults
- Clippy: pedantic + nursery at warn; module_name_repetitions allowed

## Common Issues

- **Build fails on nightly**: Ensure rust-toolchain.toml points to nightly; `rustup update nightly`
- **VCR tests fail**: Set VCR_MODE=playback VCR_CASSETTE_DIR=tests/fixtures/vcr
- **Clippy blocks CI**: Fix all warnings; CI uses -D warnings
- **Workspace default-members skip embed crates**: Validate `pi_lynx_sdk` with `-p pi_lynx_sdk` or `--workspace`, not bare `cargo check`
- **Extension JS errors**: QuickJS only — no Node/Bun APIs
- **Session lock errors**: Another pi process may hold the fs4 exclusive lock

## Debugging

- `RUST_BACKTRACE=1` for backtraces
- `RUST_LOG=debug` for tracing output
- `pi doctor` checks config, auth, sessions, compatibility
- VCR mode records/replays HTTP deterministically
- `--no-session` for throwaway testing
- Fuzz targets in `fuzz/fuzz_targets/` for parser bugs

## Ralph Integration

Subagent limits: search/read up to 100 parallel, write up to 50 parallel, build/test 1 ONLY.

# Implementation Plan - lynx-embed-sdk

> Generated: 2026-03-16
> Status: PLANNED

- [x] [P0][L][T1] Add workspace wiring in `/Users/user/dev/oss/thirdparty/pi_agent_rust/Cargo.toml`, create `crates/pi_lynx_sdk`, and set `default-members` so adding the new crate does not change the existing root package build/test/bootstrap behavior
- [x] [P0][M][T2] Define and document the embed contract in `crates/pi_lynx_sdk/src/lib.rs` and `crates/pi_lynx_sdk/src/types.rs`, including `LynxEmbedConfig`, `ProviderSelection`, `ProviderStreamOverride`, `QueueModeConfig`, transcript/result/event types, runtime metadata, history warnings, and `#[must_use]` builders where appropriate
- [x] [P0][M][T3] Implement `crates/pi_lynx_sdk/src/errors.rs` with `EmbedError` and `EmbedErrorKind`, preserving bootstrap/runtime/tool/transcript/cancellation categories, wrapped causes, and host-stable failure classification
- [x] [P0][L][T4] Implement `crates/pi_lynx_sdk/src/provider_factory.rs` to normalize host-supplied provider/model/auth inputs into `ModelEntry`, `Provider`, and conservative `StreamOptions` overrides without calling `Cli`, `Config::load()`, `AuthStorage::load_*()`, or Pi session-path bootstrap helpers
- [x] [P0][L][T5] Implement `crates/pi_lynx_sdk/src/history.rs` to reconstruct validated Pi `Message` history from host transcripts, normalize supported content blocks, preserve user/assistant/tool ordering for continue/retry flows, and surface recoverable `HistoryWarning`s versus fatal transcript errors
- [x] [P0][L][T6] Implement `crates/pi_lynx_sdk/src/tool_bridge.rs` with a private host adapter boundary, explicit `ToolPolicy` and `HostToolKind` enforcement, host-routed `Tool` implementations, streaming tool update translation, and denied/crashed tool handling that never exposes Pi built-in shell/file tools directly
- [x] [P1][L][T7] Implement `crates/pi_lynx_sdk/src/bootstrap.rs` to validate embed config, compose system prompts, default to `Session::in_memory()`, gate unsupported phase-1 session and extension modes, thread runtime metadata and queue modes into assembly, assemble host tool registries, and return `BootstrapArtifacts` separately from execution
- [x] [P1][L][T8] Implement `crates/pi_lynx_sdk/src/event_bridge.rs` and `crates/pi_lynx_sdk/src/runtime.rs` so `run_turn(...)` and `continue_turn(...)` assemble `Agent` and `AgentSession`, stay on Pi's existing async/runtime stack, wire abort signals and callbacks, translate `AgentEvent` into deterministic `EmbedEvent`s, collect optional emitted events, and return normalized `TurnResult` plus completion metadata
- [x] [P1][M][T9] Add focused tests in `crates/pi_lynx_sdk/tests/history.rs` and `crates/pi_lynx_sdk/tests/tool_bridge.rs` covering transcript ordering, invalid transcript warnings/errors, tool argument translation, streaming tool updates, host-denied tools, and host execution failures
- [x] [P1][L][T10] Add runtime/error integration tests in `crates/pi_lynx_sdk/tests/runtime_turn.rs` covering no-history turns, reconstructed-history continuation, cancellation before start/during provider streaming/during tool execution, provider construction/stream failures, `TurnFailed` event emission, and final result normalization
- [x] [P2][M][T11] Add embed-focused crate/module documentation describing host-vs-Pi ownership boundaries, phase-1 limitations, primary usage flow, and why the Lynx path bypasses `create_agent_session(...)`
- [x] [P1][S][T12] Restore the missing `legacy_pi_mono_code/pi-mono/packages/ai/src/models.generated.ts` artifact and clean up unrelated root clippy regressions so workspace compilation and targeted embed validation can run again

## Session 1

Implemented the embed-foundation slice: workspace wiring, the new `pi_lynx_sdk` crate, typed config/transcript/result/event contracts, crate-level embed errors, provider normalization, and transcript reconstruction with focused `provider_factory`/`history` nextest coverage.
Updated the spec to add `custom_type` and assistant `ToolCall` transcript blocks, and restored the missing legacy model catalog include plus a few unrelated root-clippy fixes that were blocking validation.

## Session 2

Implemented the host-routed tool bridge and bootstrap assembly layers, including the new `host_tools` config boundary, policy-gated registry construction, embed-safe denied/failed tool result translation, in-memory session/bootstrap artifact assembly, and expanded crate docs for the Lynx embed path.
Validated the slice with `cargo check -p pi_lynx_sdk`, `cargo fmt --check`, `cargo clippy -p pi_lynx_sdk --all-targets --no-deps -- -D warnings`, `cargo doc -p pi_lynx_sdk --no-deps`, and `cargo nextest run -p pi_lynx_sdk`; also updated the task spec and local agent notes to record the explicit host-tool contract and the package-scoped clippy invocation needed for this crate.

## Session 3

Implemented the runtime/event bridge slice with new `run_turn(...)`, `continue_turn(...)`, and bootstrapped execution helpers so the embed crate can execute deterministic in-process turns without relying on CLI bootstrap or remote-provider tests.
Added focused `runtime_turn` nextest coverage for no-history prompt execution, reconstructed-history continuation, pre-start cancellation, provider stream failure conversion, host-tool failure normalization, and abort handling during provider streaming and tool execution; validated with `cargo check -p pi_lynx_sdk`, `cargo fmt --check`, `cargo clippy -p pi_lynx_sdk --all-targets --no-deps -- -D warnings`, `cargo doc -p pi_lynx_sdk --no-deps`, and `cargo nextest run -p pi_lynx_sdk`.

## Session 4

Closed the review follow-up by making `reconstruct_history(...)` reject any non-tool transcript entry while assistant tool calls remain unresolved, and by failing transcripts that end before the required tool results are replayed.
Added focused `history` and `runtime_turn` nextest coverage for unresolved partial-turn transcripts and validated the package with `cargo fmt --check`, `cargo check -p pi_lynx_sdk --all-targets`, `cargo clippy -p pi_lynx_sdk --all-targets --no-deps -- -D warnings`, `cargo doc -p pi_lynx_sdk --no-deps`, and `cargo nextest run -p pi_lynx_sdk`.

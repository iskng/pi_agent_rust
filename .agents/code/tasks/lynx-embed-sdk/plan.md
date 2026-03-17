# Implementation Plan - lynx-embed-sdk

> Generated: 2026-03-16
> Status: COMPLETE

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

- [x] [P1][M][T13] Enforce tool-result replay order within a single assistant tool-use batch so reconstructed multi-tool transcripts match the sequential replay order Pi uses during `Agent::execute_tool_calls(...)`
- [x] [P1][M][T14] Make embed event capture explicitly opt-in per turn so callback-only hosts do not retain duplicated provider and tool streams in memory
- [x] [P2][S][T15] Distinguish started host tool executions from synthetic aborted/skipped completions so `tool_calls_executed` only counts tools that actually entered adapter execution

## Session 5

Closed the final two review follow-ups by making `reconstruct_history(...)` reject out-of-order multi-tool result replay and by adding an explicit `capture_events` request flag so callback-only turns no longer clone every emitted event into `TurnResult`.
Validated the completed embed crate with `cargo fmt --check`, `cargo check -p pi_lynx_sdk --all-targets`, `cargo clippy -p pi_lynx_sdk --all-targets --no-deps -- -D warnings`, and `cargo nextest run -p pi_lynx_sdk`; resolved both open `mung` issues for ordered tool replay and optional event capture.

## Session 6

Closed the remaining review regressions by scoping transcript tool-call tracking to the active unresolved assistant batch so `tool_call_id` values can be reused after a prior batch fully resolves, and by skipping policy-disabled host tools before registry validation so inactive adapters cannot fail embed bootstrap.
Validated the fixes with `cargo fmt --check`, `cargo check -p pi_lynx_sdk --all-targets`, `cargo clippy -p pi_lynx_sdk --all-targets --no-deps -- -D warnings`, `cargo nextest run -p pi_lynx_sdk --test history --test tool_bridge`, and `cargo nextest run -p pi_lynx_sdk`; resolved `mung` issues `1773706520-82145-0` and `1773706520-82148-0`.

## Session 7

Closed the last open review issue by threading an explicit started/executed bit through `AgentEvent::ToolExecutionEnd`, tracking tool-adapter entry separately from synthetic aborted/skipped completions, and teaching the Lynx embed bridge to count only actually started host tools.
Validated the fix with `cargo fmt --check`, `cargo check --all-targets`, `cargo nextest run -p pi_lynx_sdk --test runtime_turn`, `cargo nextest run --test json_mode_parity json_parity_tool_execution_end_schema`, and `cargo nextest run --test sdk_integration sdk_conformance_agent_event_json_schema`; `cargo clippy --all-targets -- -D warnings` is still blocked by unrelated pre-existing warnings in `src/extensions.rs`, `src/extensions_js.rs`, `src/interactive/*`, `src/providers/bedrock.rs`, `src/session*.rs`, and `src/sse.rs`.

## Review Follow-Up

- [x] [P1][M][T16] Preserve `BootstrapArtifacts.history_warnings` through `run_turn(...)` and `continue_turn(...)` so the high-level runtime API surfaces recoverable transcript reconstruction diagnostics instead of discarding them during execution assembly
- [x] [P1][M][T17] Stop translating synthetic skipped/aborted `AgentEvent::ToolExecutionStart`/`ToolExecutionEnd` pairs into normal embed tool lifecycle events, or thread explicit execution state through `EmbedEvent` so hosts can distinguish queued tool proposals from host adapters that actually started

## Session 8

Closed the last two review follow-ups by threading `history_warnings` into `TurnResult`, updating the embed contract/spec to document warning preservation and `ToolCompleted.executed`, and tightening `AgentEvent::ToolExecutionStart` so it fires only when a tool adapter actually begins executing.
Validated with `cargo fmt --check`, `cargo check -p pi_lynx_sdk --all-targets`, `cargo clippy -p pi_lynx_sdk --all-targets --no-deps -- -D warnings`, `cargo nextest run -p pi_lynx_sdk`, `cargo nextest run --test json_mode_parity json_parity_tool_execution_start_schema json_parity_tool_execution_end_schema`, and `cargo nextest run --test sdk_integration sdk_tool_execution sdk_conformance_tool_event_ordering`.

## Session 9

Closed the remaining open review issues by making `execute_parallel_batch()` preserve already-finished tool results when an abort interrupts a read-only batch, and by moving tool-start bookkeeping onto the exact `ToolExecutionStart` boundary so extension-hook aborts no longer report `executed: true` before adapter entry.
Also fixed a related lost-wake bug in `AbortSignal::wait()` that could hang same-thread cancellation paths, added focused root regressions for aborted parallel read-only batches and extension-hook abort timing, and validated with `cargo fmt --check`, `cargo check --all-targets`, `cargo nextest run --lib abort_during_parallel_read_only_batch_preserves_completed_outputs --status-level all --test-threads=1`, `cargo nextest run --lib abort_during_tool_call_hook_keeps_executed_false_until_adapter_entry --status-level all --test-threads=1`, and `cargo nextest run -p pi_lynx_sdk runtime_counts_only_started_tools_in_aborted_multi_tool_batches --status-level all --test-threads=1`; `cargo clippy --all-targets -- -D warnings` remains blocked by unrelated pre-existing warnings in `src/extensions.rs`, `src/extensions_js.rs`, `src/interactive/*`, `src/session*.rs`, and `src/sse.rs`.

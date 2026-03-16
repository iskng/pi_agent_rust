# Types

## Public Types

### `LynxEmbedConfig`

Top-level runtime configuration for the embed crate.

Fields:

- `provider: ProviderSelection`
- `system_prompt: Option<String>`
- `append_system_prompt: Option<String>`
- `max_tool_iterations: usize`
- `queue_mode: QueueModeConfig`
- `enable_extensions: bool`
- `session_mode: SessionMode`
- `tool_policy: ToolPolicy`
- `host_tools: Vec<Arc<dyn HostToolAdapter>>`
- `runtime_metadata: RuntimeMetadata`

Notes:

- Should not require `Cli`.
- Should not imply any on-disk config lookup by default.
- Should be explicit enough that a host can construct it from its own settings
  model without needing Pi global state.

### `ProviderSelection`

Host-supplied provider/model/auth selection input.

Fields:

- `provider_id: String`
- `model_id: String`
- `api_key: Option<String>`
- `thinking: Option<crate::model::ThinkingLevel>` or embed-specific equivalent
- `stream_options_override: Option<ProviderStreamOverride>`

Purpose:

- keep provider resolution explicit
- make provider setup deterministic in embed mode

### `ProviderStreamOverride`

Optional host-controlled overrides for provider stream behavior.

Fields may include:

- `temperature`
- `max_tokens`
- `headers`
- `reasoning_budget`

This should remain conservative and only expose fields that Pi can safely honor
across providers.

### `QueueModeConfig`

Controls how queued steering and follow-up messages are delivered during embed
turn execution.

Fields:

- `steering: crate::agent::QueueMode` or embed-specific equivalent
- `follow_up: crate::agent::QueueMode` or embed-specific equivalent

Purpose:

- let the host choose whether injected steering/follow-up messages are drained
  one-at-a-time or all-at-once
- keep queue behavior explicit instead of inheriting CLI defaults implicitly

### `SessionMode`

Embed session behavior.

Variants:

- `InMemory`
- `DebugFile(PathBuf)` or `Persistent(PathBuf)` as a future-facing option

Initial implementation default:

- `InMemory`

### `ToolPolicy`

Describes which host tool adapters are exposed to the runtime.

Fields:

- `allowed_tools: Vec<HostToolKind>`
- `allow_mutations: bool`
- `allow_exec: bool`

Purpose:

- keep tool exposure explicit and testable
- prevent accidental use of Pi built-in mutating tools
- pair policy with explicit host-owned adapter instances instead of any Pi
  built-in tool registry

### `HostToolKind`

Enumeration of host-routed tool capabilities.

Initial variants:

- `Read`
- `Search`
- `List`
- `Exec`
- `Edit`
- `Write`

The exact set can grow, but the initial spec should start with a narrow mapping
and add only what the host can policy-gate correctly.

### `RuntimeMetadata`

Opaque or semi-structured host metadata carried through a turn.

Fields:

- `conversation_id: Option<String>`
- `turn_id: Option<String>`
- `workspace_root: Option<PathBuf>`
- `user_id: Option<String>`
- `tags: BTreeMap<String, String>`

Purpose:

- enable structured tracing and audit without coupling the Pi crate directly to
  Lynx domain types

### `HostTranscriptEntry`

Host-owned transcript entry supplied to the embed runtime before each turn.

Fields:

- `role: HostTranscriptRole`
- `message_id: Option<String>`
- `tool_call_id: Option<String>`
- `tool_name: Option<String>`
- `custom_type: Option<String>`
- `content: Vec<HostContentBlock>`
- `is_error: bool`
- `timestamp_ms: Option<i64>`

Purpose:

- reconstruct Pi `Message` values from external persistence
- support retry/continue behavior without relying on Pi session files

### `HostTranscriptRole`

Variants:

- `User`
- `Assistant`
- `ToolResult`
- `Custom`

The mapping should be explicit rather than inferred from field presence.

### `HostContentBlock`

Embed-side mirror of the content shapes the host can provide.

Initial variants:

- `Text { text: String }`
- `Image { mime_type: String, data: Vec<u8> }`
- `Thinking { text: String }`
- `ToolCall { tool_call_id: String, tool_name: String, arguments: serde_json::Value }`

This should stay aligned with the Pi model layer where practical, but the embed
crate can keep its own input enum if that improves host ergonomics.

### `TurnRequest`

Primary input for a single prompt execution.

Fields:

- `config: LynxEmbedConfig`
- `transcript: Vec<HostTranscriptEntry>`
- `prompt: String`
- `on_event: Option<Arc<dyn Fn(EmbedEvent) + Send + Sync>>`
- `abort_signal: Option<AbortSignal>`

Behavior:

- reconstruct runtime state from `transcript`
- execute one user prompt
- emit translated events
- return a final normalized result

### `ContinueTurnRequest`

Input for continuing an existing reconstructed turn without injecting a new user
prompt.

Fields:

- `config: LynxEmbedConfig`
- `transcript: Vec<HostTranscriptEntry>`
- `on_event: Option<Arc<dyn Fn(EmbedEvent) + Send + Sync>>`
- `abort_signal: Option<AbortSignal>`

### `TurnResult`

Final normalized execution result returned to the host.

Fields:

- `assistant_message: crate::model::AssistantMessage`
- `stop_reason: Option<StopReason>`
- `usage: Option<Usage>`
- `emitted_events: Option<Vec<EmbedEvent>>`
- `result_metadata: TurnResultMetadata`

Purpose:

- present the host with a stable post-turn summary
- avoid forcing the host to inspect low-level Pi types beyond what is useful

### `TurnResultMetadata`

Structured metadata about the completed turn.

Fields:

- `provider_id: String`
- `model_id: String`
- `tool_calls_executed: usize`
- `had_errors: bool`
- `aborted: bool`
- `session_mode: SessionMode`

### `EmbedEvent`

Host-facing stream event emitted by the embed crate.

Initial variants:

- `TurnStarted`
- `MessageDelta { text: String }`
- `MessageCompleted { message: crate::model::Message }`
- `ToolStarted { tool_call_id: String, tool_name: String, args: serde_json::Value }`
- `ToolUpdate { tool_call_id: String, update: ToolUpdatePayload }`
- `ToolCompleted { tool_call_id: String, tool_name: String, is_error: bool }`
- `ProviderEvent { event: crate::model::StreamEvent }`
- `TurnCompleted`
- `TurnFailed { error: EmbedErrorKind }`

This is intentionally flatter and more host-oriented than raw `AgentEvent`.

### `ToolUpdatePayload`

Normalized tool update payload for host forwarding.

Fields:

- `content: Vec<crate::model::ContentBlock>`
- `details: Option<serde_json::Value>`

Purpose:

- preserve richer incremental tool payloads without collapsing them to plain
  text

## Internal Types

### `BootstrapArtifacts`

Internal assembly output from `bootstrap.rs`.

Fields:

- `session: crate::session::Session`
- `tool_registry: crate::tools::ToolRegistry`
- `agent_config: crate::agent::AgentConfig`
- `provider: Arc<dyn crate::provider::Provider>`
- `history: Vec<crate::model::Message>`
- `history_warnings: Vec<HistoryWarning>`

Purpose:

- separate validation/assembly from execution

### `HistoryConversionResult`

Output of transcript reconstruction.

Fields:

- `messages: Vec<crate::model::Message>`
- `warnings: Vec<HistoryWarning>`

Purpose:

- let the embed path tolerate minor transcript imperfections while surfacing
  diagnostics to the host or tests

### `HistoryWarning`

Variants:

- `MissingToolCallId`
- `UnsupportedContentBlock`
- `OutOfOrderToolResult`
- `DroppedInvalidEntry`

Warnings should never silently vanish inside the implementation.

### `HostToolAdapter`

Internal trait or trait-object abstraction used by `tool_bridge.rs`.

Responsibilities:

- receive normalized Pi tool calls
- delegate to host execution
- convert responses and updates back into Pi output

The exact trait shape may be private, but the module should centralize the host
tool boundary so it does not leak across the crate.

## Type Design Rules

- Public request/response types must have clear ownership and not borrow from
  ephemeral host buffers unless there is a measured performance need.
- Types should prefer concrete enums over stringly-typed state.
- Builder methods should use `#[must_use]`.
- Public enums and structs need `///` documentation.
- Errors should carry structured categories, not only rendered text.
- Embed APIs should expose the minimum Pi internals necessary for host use.
- Async-facing types should remain compatible with the project's current
  runtime model and avoid `tokio`-specific dependencies.

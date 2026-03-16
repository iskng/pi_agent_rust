//! Shared embed-facing request, response, and transcript types.

use crate::errors::EmbedErrorKind;
use crate::tool_bridge::HostToolAdapter;
use pi::model::ThinkingLevel;
use pi::sdk::{
    AbortSignal, AssistantMessage, ContentBlock, Message, QueueMode, StopReason, StreamEvent, Usage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Top-level runtime configuration for Lynx embedding.
#[derive(Debug, Clone)]
pub struct LynxEmbedConfig {
    /// Explicit provider/model/auth selection controlled by the host.
    pub provider: ProviderSelection,
    /// Optional system prompt that replaces Pi defaults.
    pub system_prompt: Option<String>,
    /// Optional system prompt suffix appended after `system_prompt`.
    pub append_system_prompt: Option<String>,
    /// Maximum number of tool loops permitted for the turn.
    pub max_tool_iterations: usize,
    /// Queue-drain behavior for steering and follow-up messages.
    pub queue_mode: QueueModeConfig,
    /// Whether Pi extensions may participate in embed mode.
    pub enable_extensions: bool,
    /// Session persistence mode for embed execution.
    pub session_mode: SessionMode,
    /// Tool exposure policy enforced by the embed layer.
    pub tool_policy: ToolPolicy,
    /// Host-routed tools available for embed execution.
    pub host_tools: Vec<Arc<dyn HostToolAdapter>>,
    /// Opaque host metadata forwarded through runtime assembly.
    pub runtime_metadata: RuntimeMetadata,
}

impl LynxEmbedConfig {
    /// Start building an embed config from a required provider selection.
    #[must_use]
    pub fn builder(provider: ProviderSelection) -> LynxEmbedConfigBuilder {
        LynxEmbedConfigBuilder {
            inner: Self {
                provider,
                system_prompt: None,
                append_system_prompt: None,
                max_tool_iterations: 50,
                queue_mode: QueueModeConfig::default(),
                enable_extensions: false,
                session_mode: SessionMode::InMemory,
                tool_policy: ToolPolicy::default(),
                host_tools: Vec::new(),
                runtime_metadata: RuntimeMetadata::default(),
            },
        }
    }
}

/// Builder for [`LynxEmbedConfig`].
#[derive(Debug, Clone)]
pub struct LynxEmbedConfigBuilder {
    inner: LynxEmbedConfig,
}

impl LynxEmbedConfigBuilder {
    /// Set the base system prompt.
    #[must_use]
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.inner.system_prompt = Some(prompt.into());
        self
    }

    /// Set the appended system prompt suffix.
    #[must_use]
    pub fn append_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.inner.append_system_prompt = Some(prompt.into());
        self
    }

    /// Override the tool-iteration limit.
    #[must_use]
    pub fn max_tool_iterations(mut self, max_tool_iterations: usize) -> Self {
        self.inner.max_tool_iterations = max_tool_iterations;
        self
    }

    /// Override queue delivery behavior.
    #[must_use]
    pub fn queue_mode(mut self, queue_mode: QueueModeConfig) -> Self {
        self.inner.queue_mode = queue_mode;
        self
    }

    /// Enable or disable Pi extensions for embed mode.
    #[must_use]
    pub fn enable_extensions(mut self, enable_extensions: bool) -> Self {
        self.inner.enable_extensions = enable_extensions;
        self
    }

    /// Override the embed session mode.
    #[must_use]
    pub fn session_mode(mut self, session_mode: SessionMode) -> Self {
        self.inner.session_mode = session_mode;
        self
    }

    /// Override the tool policy.
    #[must_use]
    pub fn tool_policy(mut self, tool_policy: ToolPolicy) -> Self {
        self.inner.tool_policy = tool_policy;
        self
    }

    /// Replace the host-routed tool set.
    #[must_use]
    pub fn host_tools<I>(mut self, host_tools: I) -> Self
    where
        I: IntoIterator<Item = Arc<dyn HostToolAdapter>>,
    {
        self.inner.host_tools = host_tools.into_iter().collect();
        self
    }

    /// Append a single host-routed tool adapter.
    #[must_use]
    pub fn push_host_tool(mut self, host_tool: Arc<dyn HostToolAdapter>) -> Self {
        self.inner.host_tools.push(host_tool);
        self
    }

    /// Attach structured host metadata.
    #[must_use]
    pub fn runtime_metadata(mut self, runtime_metadata: RuntimeMetadata) -> Self {
        self.inner.runtime_metadata = runtime_metadata;
        self
    }

    /// Finalize the embed configuration.
    #[must_use]
    pub fn build(self) -> LynxEmbedConfig {
        self.inner
    }
}

/// Host-controlled provider/model/auth selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSelection {
    /// Provider identifier or alias.
    pub provider_id: String,
    /// Provider-native model identifier.
    pub model_id: String,
    /// Optional API key override supplied directly by the host.
    pub api_key: Option<String>,
    /// Optional reasoning/thinking preference.
    pub thinking: Option<ThinkingLevel>,
    /// Optional per-request stream overrides.
    #[serde(default)]
    pub stream_options_override: Option<ProviderStreamOverride>,
}

/// Conservative host overrides for provider stream behavior.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStreamOverride {
    /// Optional temperature override.
    pub temperature: Option<f32>,
    /// Optional max-token override.
    pub max_tokens: Option<u32>,
    /// Optional extra headers to attach per request.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Optional uniform reasoning-budget override in tokens.
    pub reasoning_budget_tokens: Option<u32>,
}

impl ProviderStreamOverride {
    /// Set a numeric temperature override.
    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

/// Explicit queue mode choices for embed turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueModeConfig {
    /// Delivery mode for steering messages.
    pub steering: QueueMode,
    /// Delivery mode for follow-up messages.
    pub follow_up: QueueMode,
}

impl Default for QueueModeConfig {
    fn default() -> Self {
        Self {
            steering: QueueMode::OneAtATime,
            follow_up: QueueMode::OneAtATime,
        }
    }
}

/// Session persistence mode for embed execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionMode {
    /// Purely in-memory session execution.
    InMemory,
    /// Optional file-backed mode reserved for diagnostics.
    DebugFile(PathBuf),
    /// Optional persistent mode reserved for future expansion.
    Persistent(PathBuf),
}

/// Policy describing which host-routed tools are exposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicy {
    /// Whitelisted host-routed tool kinds.
    pub allowed_tools: Vec<HostToolKind>,
    /// Whether file mutations are allowed at all.
    pub allow_mutations: bool,
    /// Whether exec-capable tools are allowed at all.
    pub allow_exec: bool,
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            allowed_tools: vec![HostToolKind::Read, HostToolKind::Search, HostToolKind::List],
            allow_mutations: false,
            allow_exec: false,
        }
    }
}

impl ToolPolicy {
    /// Return whether a specific host-routed tool kind is permitted.
    #[must_use]
    pub fn allows(&self, tool: HostToolKind) -> bool {
        self.allowed_tools.contains(&tool)
            && (!matches!(tool, HostToolKind::Edit | HostToolKind::Write) || self.allow_mutations)
            && (!matches!(tool, HostToolKind::Exec) || self.allow_exec)
    }
}

/// Host-routed tool capabilities surfaced to the embed runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostToolKind {
    /// Read file contents.
    Read,
    /// Search file contents.
    Search,
    /// List directories or files.
    List,
    /// Execute processes through host policy.
    Exec,
    /// Edit files through host policy.
    Edit,
    /// Write files through host policy.
    Write,
}

impl HostToolKind {
    /// Return whether the tool kind is safe to run in parallel.
    #[must_use]
    pub const fn is_read_only(self) -> bool {
        matches!(self, Self::Read | Self::Search | Self::List)
    }

    /// Return whether the tool kind mutates workspace state.
    #[must_use]
    pub const fn is_mutating(self) -> bool {
        matches!(self, Self::Edit | Self::Write)
    }

    /// Return whether the tool kind may execute subprocesses.
    #[must_use]
    pub const fn is_exec(self) -> bool {
        matches!(self, Self::Exec)
    }
}

/// Structured metadata passed through embed runtime assembly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetadata {
    /// Host conversation identifier.
    pub conversation_id: Option<String>,
    /// Host turn identifier.
    pub turn_id: Option<String>,
    /// Optional workspace root visible to the host.
    pub workspace_root: Option<PathBuf>,
    /// Optional user identifier for audit/tracing.
    pub user_id: Option<String>,
    /// Arbitrary host-provided tags.
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

/// Host-owned transcript entry reconstructed before each turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostTranscriptEntry {
    /// Host-defined message role.
    pub role: HostTranscriptRole,
    /// Optional host message identifier.
    pub message_id: Option<String>,
    /// Optional tool call identifier used by tool-result entries.
    pub tool_call_id: Option<String>,
    /// Optional tool name used by tool-result entries.
    pub tool_name: Option<String>,
    /// Optional custom type for `Custom` role entries.
    pub custom_type: Option<String>,
    /// Structured content blocks for the entry.
    pub content: Vec<HostContentBlock>,
    /// Whether the transcript entry semantically represents an error.
    pub is_error: bool,
    /// Optional wall-clock timestamp in milliseconds.
    pub timestamp_ms: Option<i64>,
}

/// Host-owned transcript role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostTranscriptRole {
    /// User-authored message.
    User,
    /// Assistant-authored message.
    Assistant,
    /// Tool-result message.
    ToolResult,
    /// Host-defined message type.
    Custom,
}

/// Host-supplied content block used for transcript reconstruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostContentBlock {
    /// Plain text content.
    Text { text: String },
    /// Inline image bytes.
    Image { mime_type: String, data: Vec<u8> },
    /// Reasoning/thinking content.
    Thinking { text: String },
    /// Tool call emitted by a prior assistant message.
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
    },
}

/// Input for a single prompt execution.
#[derive(Clone)]
pub struct TurnRequest {
    /// Embed runtime configuration.
    pub config: LynxEmbedConfig,
    /// Host-owned transcript reconstructed before the turn.
    pub transcript: Vec<HostTranscriptEntry>,
    /// User prompt injected for this turn.
    pub prompt: String,
    /// Optional event callback.
    pub on_event: Option<Arc<dyn Fn(EmbedEvent) + Send + Sync>>,
    /// Optional abort signal.
    pub abort_signal: Option<AbortSignal>,
}

/// Input for resuming a reconstructed turn without a new user prompt.
#[derive(Clone)]
pub struct ContinueTurnRequest {
    /// Embed runtime configuration.
    pub config: LynxEmbedConfig,
    /// Host-owned transcript reconstructed before the turn.
    pub transcript: Vec<HostTranscriptEntry>,
    /// Optional event callback.
    pub on_event: Option<Arc<dyn Fn(EmbedEvent) + Send + Sync>>,
    /// Optional abort signal.
    pub abort_signal: Option<AbortSignal>,
}

/// Final normalized result returned to the host after a turn.
#[derive(Debug, Clone)]
pub struct TurnResult {
    /// Final assistant message emitted by Pi.
    pub assistant_message: AssistantMessage,
    /// Stop reason for the completed turn, when available.
    pub stop_reason: Option<StopReason>,
    /// Usage metadata for the completed turn, when available.
    pub usage: Option<Usage>,
    /// Optional recorded event stream when the host asks for it.
    pub emitted_events: Option<Vec<EmbedEvent>>,
    /// Structured completion metadata.
    pub result_metadata: TurnResultMetadata,
}

/// Structured metadata describing a completed turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResultMetadata {
    /// Provider used for the turn.
    pub provider_id: String,
    /// Model used for the turn.
    pub model_id: String,
    /// Number of host tools executed during the turn.
    pub tool_calls_executed: usize,
    /// Whether any errors were observed during the turn.
    pub had_errors: bool,
    /// Whether the turn ended due to cancellation.
    pub aborted: bool,
    /// Session mode used for the turn.
    pub session_mode: SessionMode,
}

/// Host-facing event emitted by the embed runtime.
#[derive(Debug, Clone)]
pub enum EmbedEvent {
    /// Turn execution is starting.
    TurnStarted,
    /// Assistant text delta.
    MessageDelta { text: String },
    /// Message completed.
    MessageCompleted { message: Message },
    /// Tool execution started.
    ToolStarted {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    /// Tool emitted an incremental update.
    ToolUpdate {
        tool_call_id: String,
        update: ToolUpdatePayload,
    },
    /// Tool execution completed.
    ToolCompleted {
        tool_call_id: String,
        tool_name: String,
        is_error: bool,
    },
    /// Raw provider stream event forwarded to the host.
    ProviderEvent { event: StreamEvent },
    /// Turn execution completed successfully.
    TurnCompleted,
    /// Turn execution failed.
    TurnFailed { error: EmbedErrorKind },
}

/// Normalized tool update payload surfaced to hosts.
#[derive(Debug, Clone)]
pub struct ToolUpdatePayload {
    /// Incremental content emitted by the tool.
    pub content: Vec<ContentBlock>,
    /// Optional structured details payload.
    pub details: Option<Value>,
}

/// Recoverable warning encountered during transcript reconstruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryWarning {
    /// Stable warning category.
    pub kind: HistoryWarningKind,
    /// Host message identifier when available.
    pub message_id: Option<String>,
    /// Human-readable warning detail for diagnostics.
    pub detail: String,
}

/// Stable warning kinds returned by transcript reconstruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryWarningKind {
    /// Non-text custom content was dropped because Pi custom messages are text-only.
    CustomContentBlockDropped,
}

/// Transcript reconstruction output.
#[derive(Debug, Clone)]
pub struct HistoryConversionResult {
    /// Reconstructed Pi messages in transcript order.
    pub messages: Vec<Message>,
    /// Recoverable warnings encountered during reconstruction.
    pub warnings: Vec<HistoryWarning>,
}

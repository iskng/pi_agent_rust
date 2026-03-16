//! Lynx-oriented embedding surface for Pi runtime assembly.
//!
//! This crate exists to give hosts such as Lynx an explicit in-process
//! integration boundary that bypasses Pi's CLI-shaped bootstrap path. The host
//! remains the source of truth for persistence, permissions, and tool routing;
//! Pi supplies the lower-level runtime primitives that will eventually execute
//! turns against provider backends.
//!
//! Unlike [`pi::sdk::create_agent_session`], this crate does not load Pi global
//! config, Pi auth storage, Pi-owned session files, or Pi built-in shell/file
//! tools. That is intentional: Lynx owns persistence, permissions, and tool
//! transport, while Pi only owns provider dispatch, transcript normalization,
//! and the lower-level agent loop.
//!
//! Primary phase-1 flow:
//!
//! 1. Build [`LynxEmbedConfig`] with explicit provider selection and host tool
//!    adapters.
//! 2. Reconstruct prior transcript state with [`reconstruct_history`] or let
//!    [`bootstrap_turn`] do it during assembly.
//! 3. Call [`bootstrap_turn`] to produce in-memory session, provider, tool
//!    registry, and agent config artifacts.
//! 4. Hand those artifacts to a runtime runner that executes a single turn.
//!
//! Phase-1 limitations:
//!
//! - only in-memory sessions are supported
//! - Pi extensions are intentionally disabled
//! - only host-routed tools are exposed
//! - this path avoids `Cli`, `Config::load()`, and Pi-owned session bootstrap
//!   helpers

pub mod bootstrap;
pub mod errors;
pub mod history;
pub mod provider_factory;
pub mod tool_bridge;
pub mod types;

pub use crate::bootstrap::{BootstrapArtifacts, bootstrap_turn};
pub use crate::errors::{EmbedError, EmbedErrorKind, Result};
pub use crate::history::reconstruct_history;
pub use crate::provider_factory::{
    ResolvedProvider, build_stream_options, resolve_model_entry, resolve_provider,
};
pub use crate::tool_bridge::{
    HostToolAdapter, HostToolDefinition, HostToolError, HostToolErrorKind, HostToolOutput,
    HostToolRequest, HostToolUpdate, build_tool_registry,
};
pub use crate::types::{
    ContinueTurnRequest, EmbedEvent, HistoryConversionResult, HistoryWarning, HistoryWarningKind,
    HostContentBlock, HostToolKind, HostTranscriptEntry, HostTranscriptRole, LynxEmbedConfig,
    LynxEmbedConfigBuilder, ProviderSelection, ProviderStreamOverride, QueueModeConfig,
    RuntimeMetadata, SessionMode, ToolPolicy, ToolUpdatePayload, TurnRequest, TurnResult,
    TurnResultMetadata,
};

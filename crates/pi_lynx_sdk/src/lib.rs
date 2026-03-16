//! Lynx-oriented embedding surface for Pi runtime assembly.
//!
//! This crate exists to give hosts such as Lynx an explicit in-process
//! integration boundary that bypasses Pi's CLI-shaped bootstrap path. The host
//! remains the source of truth for persistence, permissions, and tool routing;
//! Pi supplies the lower-level runtime primitives that will eventually execute
//! turns against provider backends.
//!
//! Phase 1 in this crate focuses on explicit contract types, provider
//! resolution, and transcript reconstruction. Runtime/bootstrap/tool bridging
//! layers can build on these primitives without pulling in `Cli`,
//! `Config::load()`, or Pi-owned session bootstrap helpers.

pub mod errors;
pub mod history;
pub mod provider_factory;
pub mod types;

pub use crate::errors::{EmbedError, EmbedErrorKind, Result};
pub use crate::history::reconstruct_history;
pub use crate::provider_factory::{
    ResolvedProvider, build_stream_options, resolve_model_entry, resolve_provider,
};
pub use crate::types::{
    ContinueTurnRequest, EmbedEvent, HistoryConversionResult, HistoryWarning, HistoryWarningKind,
    HostContentBlock, HostToolKind, HostTranscriptEntry, HostTranscriptRole, LynxEmbedConfig,
    LynxEmbedConfigBuilder, ProviderSelection, ProviderStreamOverride, QueueModeConfig,
    RuntimeMetadata, SessionMode, ToolPolicy, ToolUpdatePayload, TurnRequest, TurnResult,
    TurnResultMetadata,
};

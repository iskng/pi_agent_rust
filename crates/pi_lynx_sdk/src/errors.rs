//! Error boundary for the Lynx embedding crate.

use pi::sdk::Error as PiError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type alias for the Lynx embedding crate.
pub type Result<T> = std::result::Result<T, EmbedError>;

/// Stable machine-readable failure classes exposed to hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbedErrorKind {
    InvalidConfig,
    InvalidTranscript,
    ProviderUnavailable,
    ProviderStreamFailed,
    ToolDenied,
    ToolFailed,
    Cancelled,
    Internal,
}

/// Host-facing error boundary for Lynx embedding operations.
#[derive(Debug, Error)]
pub enum EmbedError {
    /// Embed configuration was invalid before execution could start.
    #[error("Invalid embed configuration during {operation}: {message}")]
    Config {
        operation: &'static str,
        message: String,
    },

    /// Runtime assembly failed before the agent loop began.
    #[error("Bootstrap failed during {operation}: {source}")]
    Bootstrap {
        operation: &'static str,
        provider_id: Option<String>,
        model_id: Option<String>,
        #[source]
        source: Box<PiError>,
    },

    /// Host transcript could not be converted into Pi message history.
    #[error("Transcript reconstruction failed: {message}")]
    Transcript { message: String },

    /// Provider creation or streaming failed.
    #[error("Provider execution failed for {provider_id}/{model_id}: {source}")]
    Provider {
        provider_id: String,
        model_id: String,
        #[source]
        source: Box<PiError>,
    },

    /// Tool execution was denied by host policy.
    #[error("Tool '{tool_name}' was denied: {message}")]
    ToolDenied { tool_name: String, message: String },

    /// Tool execution failed after it was accepted.
    #[error("Tool '{tool_name}' failed: {message}")]
    Tool {
        tool_name: String,
        message: String,
        #[source]
        source: Option<Box<PiError>>,
    },

    /// Session state could not be created or updated.
    #[error("Session setup failed during {operation}: {source}")]
    Session {
        operation: &'static str,
        #[source]
        source: Box<PiError>,
    },

    /// Event translation or delivery failed.
    #[error("Event bridge failed: {message}")]
    EventBridge { message: String },

    /// Host or user cancellation stopped the turn.
    #[error("Embed turn aborted")]
    Aborted,

    /// Unexpected invariant breakage inside the embed crate.
    #[error("Internal embed runtime error: {message}")]
    Internal { message: String },
}

impl EmbedError {
    /// Construct an invalid-config error with operation context.
    #[must_use]
    pub fn config(operation: &'static str, message: impl Into<String>) -> Self {
        Self::Config {
            operation,
            message: message.into(),
        }
    }

    /// Construct a bootstrap failure with provider/model context when known.
    #[must_use]
    pub fn bootstrap(
        operation: &'static str,
        provider_id: Option<String>,
        model_id: Option<String>,
        source: PiError,
    ) -> Self {
        Self::Bootstrap {
            operation,
            provider_id,
            model_id,
            source: Box::new(source),
        }
    }

    /// Construct a transcript reconstruction error.
    #[must_use]
    pub fn transcript(message: impl Into<String>) -> Self {
        Self::Transcript {
            message: message.into(),
        }
    }

    /// Construct a provider failure with provider/model context.
    #[must_use]
    pub fn provider(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        source: PiError,
    ) -> Self {
        Self::Provider {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            source: Box::new(source),
        }
    }

    /// Construct a tool-denied failure.
    #[must_use]
    pub fn tool_denied(tool_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ToolDenied {
            tool_name: tool_name.into(),
            message: message.into(),
        }
    }

    /// Construct a tool execution failure.
    #[must_use]
    pub fn tool_failed(
        tool_name: impl Into<String>,
        message: impl Into<String>,
        source: Option<PiError>,
    ) -> Self {
        Self::Tool {
            tool_name: tool_name.into(),
            message: message.into(),
            source: source.map(Box::new),
        }
    }

    /// Construct a session failure.
    #[must_use]
    pub fn session(operation: &'static str, source: PiError) -> Self {
        Self::Session {
            operation,
            source: Box::new(source),
        }
    }

    /// Construct an event bridge failure.
    #[must_use]
    pub fn event_bridge(message: impl Into<String>) -> Self {
        Self::EventBridge {
            message: message.into(),
        }
    }

    /// Construct an internal failure.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Return the stable machine-readable error kind for this failure.
    #[must_use]
    pub const fn kind(&self) -> EmbedErrorKind {
        match self {
            Self::Config { .. } => EmbedErrorKind::InvalidConfig,
            Self::Transcript { .. } => EmbedErrorKind::InvalidTranscript,
            Self::Bootstrap { .. } => EmbedErrorKind::ProviderUnavailable,
            Self::Provider { .. } => EmbedErrorKind::ProviderStreamFailed,
            Self::ToolDenied { .. } => EmbedErrorKind::ToolDenied,
            Self::Tool { .. } => EmbedErrorKind::ToolFailed,
            Self::Aborted => EmbedErrorKind::Cancelled,
            Self::Session { .. } | Self::EventBridge { .. } | Self::Internal { .. } => {
                EmbedErrorKind::Internal
            }
        }
    }
}

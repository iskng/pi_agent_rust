//! Host-routed tool adapters for Lynx embed mode.

use crate::errors::{EmbedError, Result};
use crate::types::{HostToolKind, RuntimeMetadata, ToolPolicy};
use async_trait::async_trait;
use pi::sdk::{ContentBlock, Tool, ToolOutput, ToolRegistry, ToolUpdate};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

/// Stable description of a host-routed tool exposed to Pi.
#[derive(Debug, Clone)]
pub struct HostToolDefinition {
    /// Stable tool name visible to the model.
    pub name: String,
    /// Human-facing label used in UI/event surfaces.
    pub label: String,
    /// Tool purpose and usage guidance.
    pub description: String,
    /// JSON Schema parameters accepted by the tool.
    pub parameters: Value,
}

/// Host-owned tool request normalized by the embed bridge.
#[derive(Debug, Clone)]
pub struct HostToolRequest {
    /// Provider-supplied tool call identifier.
    pub tool_call_id: String,
    /// Stable tool name visible to the model.
    pub tool_name: String,
    /// Host tool capability kind enforced by policy.
    pub kind: HostToolKind,
    /// Raw JSON arguments emitted by the model.
    pub input: Value,
    /// Opaque host metadata forwarded from embed config.
    pub runtime_metadata: RuntimeMetadata,
}

/// Host-owned tool output returned on successful execution.
#[derive(Debug, Clone)]
pub struct HostToolOutput {
    /// Content blocks to surface back into Pi's transcript.
    pub content: Vec<ContentBlock>,
    /// Optional structured details payload.
    pub details: Option<Value>,
    /// Whether the tool completed with a semantic error result.
    pub is_error: bool,
}

/// Incremental update emitted while a host tool is still running.
#[derive(Debug, Clone)]
pub struct HostToolUpdate {
    /// Partial content emitted by the host tool.
    pub content: Vec<ContentBlock>,
    /// Optional structured details payload.
    pub details: Option<Value>,
}

/// Stable classification for host-routed tool failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostToolErrorKind {
    /// Host policy denied execution after the model requested the tool.
    Denied,
    /// Host transport/execution/serialization failed after acceptance.
    Failed,
}

/// Structured host-routed tool failure returned by adapters.
#[derive(Debug, Clone)]
pub struct HostToolError {
    kind: HostToolErrorKind,
    message: String,
    details: Option<Value>,
}

impl HostToolError {
    /// Construct a policy-denied host tool failure.
    #[must_use]
    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            kind: HostToolErrorKind::Denied,
            message: message.into(),
            details: None,
        }
    }

    /// Construct a policy-denied host tool failure with structured details.
    #[must_use]
    pub fn denied_with_details(message: impl Into<String>, details: Value) -> Self {
        Self {
            kind: HostToolErrorKind::Denied,
            message: message.into(),
            details: Some(details),
        }
    }

    /// Construct a host execution failure.
    #[must_use]
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            kind: HostToolErrorKind::Failed,
            message: message.into(),
            details: None,
        }
    }

    /// Construct a host execution failure with structured details.
    #[must_use]
    pub fn failed_with_details(message: impl Into<String>, details: Value) -> Self {
        Self {
            kind: HostToolErrorKind::Failed,
            message: message.into(),
            details: Some(details),
        }
    }

    /// Return the stable failure classification.
    #[must_use]
    pub const fn kind(&self) -> HostToolErrorKind {
        self.kind
    }

    /// Return the human-facing failure message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Return the optional structured details payload.
    #[must_use]
    pub const fn details(&self) -> Option<&Value> {
        self.details.as_ref()
    }

    fn into_tool_output(self) -> ToolOutput {
        let prefix = match self.kind {
            HostToolErrorKind::Denied => "Denied",
            HostToolErrorKind::Failed => "Error",
        };
        let details = match self.details {
            Some(Value::Object(mut map)) => {
                map.entry("hostToolErrorKind".to_string())
                    .or_insert_with(|| {
                        Value::String(
                            match self.kind {
                                HostToolErrorKind::Denied => "denied",
                                HostToolErrorKind::Failed => "failed",
                            }
                            .to_string(),
                        )
                    });
                Some(Value::Object(map))
            }
            Some(other) => Some(json!({
                "hostToolErrorKind": match self.kind {
                    HostToolErrorKind::Denied => "denied",
                    HostToolErrorKind::Failed => "failed",
                },
                "hostToolDetails": other,
            })),
            None => Some(json!({
                "hostToolErrorKind": match self.kind {
                    HostToolErrorKind::Denied => "denied",
                    HostToolErrorKind::Failed => "failed",
                },
            })),
        };

        ToolOutput {
            content: vec![ContentBlock::Text(pi::sdk::TextContent::new(format!(
                "{prefix}: {}",
                self.message
            )))],
            details,
            is_error: true,
        }
    }
}

impl fmt::Display for HostToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for HostToolError {}

/// Host-owned adapter boundary used to expose tools into embed mode.
#[async_trait]
pub trait HostToolAdapter: Send + Sync + fmt::Debug {
    /// Return the capability kind used for embed policy enforcement.
    fn kind(&self) -> HostToolKind;

    /// Return the model-visible definition for this tool.
    fn definition(&self) -> HostToolDefinition;

    /// Execute the host-owned tool request.
    async fn execute(
        &self,
        request: HostToolRequest,
        on_update: Option<Box<dyn Fn(HostToolUpdate) + Send + Sync>>,
    ) -> std::result::Result<HostToolOutput, HostToolError>;
}

/// Build a Pi [`ToolRegistry`] from host-routed adapters.
pub fn build_tool_registry(
    policy: &ToolPolicy,
    runtime_metadata: &RuntimeMetadata,
    host_tools: &[Arc<dyn HostToolAdapter>],
) -> Result<ToolRegistry> {
    let mut names = BTreeSet::new();
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    for adapter in host_tools {
        let definition = adapter.definition();
        validate_definition(&definition, adapter.kind())?;

        if !names.insert(definition.name.clone()) {
            return Err(EmbedError::config(
                "tool_bridge::build_tool_registry",
                format!("duplicate host tool name '{}'", definition.name),
            ));
        }

        if !policy.allows(adapter.kind()) {
            tracing::debug!(
                tool_name = %definition.name,
                tool_kind = ?adapter.kind(),
                "Skipping host tool because embed policy does not allow it"
            );
            continue;
        }

        tools.push(Box::new(HostToolWrapper {
            adapter: Arc::clone(adapter),
            definition,
            runtime_metadata: runtime_metadata.clone(),
        }));
    }

    Ok(ToolRegistry::from_tools(tools))
}

fn validate_definition(definition: &HostToolDefinition, kind: HostToolKind) -> Result<()> {
    if definition.name.trim().is_empty() {
        return Err(EmbedError::config(
            "tool_bridge::validate_definition",
            format!("host tool definition for {kind:?} is missing a name"),
        ));
    }
    if definition.label.trim().is_empty() {
        return Err(EmbedError::config(
            "tool_bridge::validate_definition",
            format!("host tool '{}' is missing a label", definition.name),
        ));
    }
    if definition.description.trim().is_empty() {
        return Err(EmbedError::config(
            "tool_bridge::validate_definition",
            format!("host tool '{}' is missing a description", definition.name),
        ));
    }
    if !definition.parameters.is_object() {
        return Err(EmbedError::config(
            "tool_bridge::validate_definition",
            format!(
                "host tool '{}' parameters must be a JSON object schema",
                definition.name
            ),
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct HostToolWrapper {
    adapter: Arc<dyn HostToolAdapter>,
    definition: HostToolDefinition,
    runtime_metadata: RuntimeMetadata,
}

#[async_trait]
#[allow(clippy::unnecessary_literal_bound)]
impl Tool for HostToolWrapper {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn label(&self) -> &str {
        &self.definition.label
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn parameters(&self) -> Value {
        self.definition.parameters.clone()
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        input: Value,
        on_update: Option<Box<dyn Fn(ToolUpdate) + Send + Sync>>,
    ) -> pi::sdk::Result<ToolOutput> {
        let on_update = on_update.map(|on_update| {
            Box::new(move |update: HostToolUpdate| {
                on_update(ToolUpdate {
                    content: update.content,
                    details: update.details,
                });
            }) as Box<dyn Fn(HostToolUpdate) + Send + Sync>
        });

        let request = HostToolRequest {
            tool_call_id: tool_call_id.to_string(),
            tool_name: self.definition.name.clone(),
            kind: self.adapter.kind(),
            input,
            runtime_metadata: self.runtime_metadata.clone(),
        };

        match self.adapter.execute(request, on_update).await {
            Ok(output) => Ok(ToolOutput {
                content: output.content,
                details: output.details,
                is_error: output.is_error,
            }),
            Err(error) => {
                tracing::warn!(
                    tool_name = %self.definition.name,
                    tool_kind = ?self.adapter.kind(),
                    error_kind = ?error.kind(),
                    error = %error,
                    "Host tool adapter returned an embed-safe failure"
                );
                Ok(error.into_tool_output())
            }
        }
    }

    fn is_read_only(&self) -> bool {
        self.adapter.kind().is_read_only()
    }
}

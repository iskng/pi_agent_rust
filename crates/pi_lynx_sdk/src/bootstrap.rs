//! Bootstrap assembly for Lynx embed turns.

use crate::errors::{EmbedError, Result};
use crate::history::reconstruct_history;
use crate::provider_factory::resolve_provider;
use crate::tool_bridge::build_tool_registry;
use crate::types::{HistoryWarning, LynxEmbedConfig, SessionMode};
use pi::sdk::{AgentConfig, Message, Provider, Session, StreamOptions, ToolRegistry};
use std::fmt;
use std::sync::Arc;

const EMBED_DEFAULT_SYSTEM_PROMPT: &str = concat!(
    "You are Pi running inside an embedded Lynx host. ",
    "Conversation persistence, permissions, and tool execution are owned by the host. ",
    "Use only the tools explicitly provided in this session."
);

/// Fully assembled runtime inputs produced before execution begins.
pub struct BootstrapArtifacts {
    /// In-memory Pi session reconstructed from host transcript history.
    pub session: Session,
    /// Host-routed tool registry exposed to the agent loop.
    pub tool_registry: ToolRegistry,
    /// Agent configuration used for turn execution.
    pub agent_config: AgentConfig,
    /// Resolved Pi provider for this embed turn.
    pub provider: Arc<dyn Provider>,
    /// Reconstructed message history in transcript order.
    pub history: Vec<Message>,
    /// Recoverable transcript warnings preserved for host diagnostics.
    pub history_warnings: Vec<HistoryWarning>,
}

impl fmt::Debug for BootstrapArtifacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BootstrapArtifacts")
            .field("session_id", &self.session.header.id)
            .field("tool_count", &self.tool_registry.tools().len())
            .field("provider", &self.provider.name())
            .field("model_id", &self.provider.model_id())
            .field("history_len", &self.history.len())
            .field("history_warning_count", &self.history_warnings.len())
            .finish()
    }
}

/// Validate embed config and assemble Pi runtime primitives for one turn.
pub fn bootstrap_turn(
    config: &LynxEmbedConfig,
    transcript: &[crate::types::HostTranscriptEntry],
) -> Result<BootstrapArtifacts> {
    validate_config(config)?;

    let resolved_provider = resolve_provider(&config.provider)?;
    let history_result = reconstruct_history(transcript)?;
    let tool_registry = build_tool_registry(
        &config.tool_policy,
        &config.runtime_metadata,
        &config.host_tools,
    )?;
    let provider_id = resolved_provider.provider_id().to_string();
    let model_id = resolved_provider.model_id().to_string();

    let mut session = Session::in_memory();
    session.set_model_header(
        Some(provider_id.clone()),
        Some(model_id.clone()),
        config
            .provider
            .thinking
            .map(|thinking| thinking.to_string()),
    );
    if let Some(workspace_root) = &config.runtime_metadata.workspace_root {
        session.header.cwd = workspace_root.display().to_string();
    }

    for message in &history_result.messages {
        session.append_model_message(message.clone());
    }

    let agent_config = AgentConfig {
        system_prompt: Some(compose_system_prompt(config)),
        max_tool_iterations: config.max_tool_iterations,
        stream_options: with_session_id(&session.header.id, resolved_provider.stream_options),
        block_images: false,
    };

    tracing::debug!(
        provider = %provider_id,
        model_id = %model_id,
        history_len = history_result.messages.len(),
        tool_count = tool_registry.tools().len(),
        "Bootstrapped Lynx embed artifacts"
    );

    Ok(BootstrapArtifacts {
        session,
        tool_registry,
        agent_config,
        provider: resolved_provider.provider,
        history: history_result.messages,
        history_warnings: history_result.warnings,
    })
}

fn validate_config(config: &LynxEmbedConfig) -> Result<()> {
    if config.max_tool_iterations == 0 {
        return Err(EmbedError::config(
            "bootstrap::validate_config",
            "max_tool_iterations must be greater than zero",
        ));
    }

    if config.enable_extensions {
        return Err(EmbedError::config(
            "bootstrap::validate_config",
            "embed mode does not support Pi extensions in phase 1",
        ));
    }

    if !matches!(config.session_mode, SessionMode::InMemory) {
        return Err(EmbedError::config(
            "bootstrap::validate_config",
            "embed mode currently supports only SessionMode::InMemory",
        ));
    }

    if config
        .tool_policy
        .allowed_tools
        .iter()
        .any(|tool| tool.is_mutating())
        && !config.tool_policy.allow_mutations
    {
        return Err(EmbedError::config(
            "bootstrap::validate_config",
            "tool_policy allows mutating tools without allow_mutations=true",
        ));
    }

    if config
        .tool_policy
        .allowed_tools
        .iter()
        .any(|tool| tool.is_exec())
        && !config.tool_policy.allow_exec
    {
        return Err(EmbedError::config(
            "bootstrap::validate_config",
            "tool_policy allows exec tools without allow_exec=true",
        ));
    }

    if let Some(workspace_root) = &config.runtime_metadata.workspace_root
        && !workspace_root.is_absolute()
    {
        return Err(EmbedError::config(
            "bootstrap::validate_config",
            "runtime_metadata.workspace_root must be absolute when provided",
        ));
    }

    Ok(())
}

fn compose_system_prompt(config: &LynxEmbedConfig) -> String {
    let mut prompt = config
        .system_prompt
        .clone()
        .unwrap_or_else(|| EMBED_DEFAULT_SYSTEM_PROMPT.to_string());

    if let Some(append) = config
        .append_system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|append| !append.is_empty())
    {
        if !prompt.trim().is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(append);
    }

    prompt
}

fn with_session_id(session_id: &str, mut stream_options: StreamOptions) -> StreamOptions {
    stream_options.session_id = Some(session_id.to_string());
    stream_options
}

#[cfg(test)]
mod tests {
    use super::{EMBED_DEFAULT_SYSTEM_PROMPT, bootstrap_turn};
    use crate::tool_bridge::{
        HostToolAdapter, HostToolDefinition, HostToolOutput, HostToolRequest,
    };
    use crate::types::{
        HostToolKind, LynxEmbedConfig, ProviderSelection, RuntimeMetadata, ToolPolicy,
    };
    use async_trait::async_trait;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::env;
    use std::sync::Arc;

    #[derive(Debug)]
    struct StaticTool {
        kind: HostToolKind,
        definition: HostToolDefinition,
    }

    #[async_trait]
    impl HostToolAdapter for StaticTool {
        fn kind(&self) -> HostToolKind {
            self.kind
        }

        fn definition(&self) -> HostToolDefinition {
            self.definition.clone()
        }

        async fn execute(
            &self,
            _request: HostToolRequest,
            _on_update: Option<Box<dyn Fn(crate::tool_bridge::HostToolUpdate) + Send + Sync>>,
        ) -> std::result::Result<HostToolOutput, crate::tool_bridge::HostToolError> {
            Ok(HostToolOutput {
                content: Vec::new(),
                details: None,
                is_error: false,
            })
        }
    }

    /// WHY: bootstrap must reject unsupported phase-1 modes up front so hosts
    /// do not accidentally fall back to Pi-owned persistence or extension boot.
    #[test]
    fn bootstrap_rejects_unsupported_modes() {
        let config = LynxEmbedConfig::builder(sample_provider())
            .enable_extensions(true)
            .build();

        let error = bootstrap_turn(&config, &[]).expect_err("extensions must be rejected");
        assert_eq!(error.kind(), crate::EmbedErrorKind::InvalidConfig);
        assert!(error.to_string().contains("does not support Pi extensions"));
    }

    /// WHY: bootstrap needs to carry reconstructed history and session metadata
    /// into an in-memory session before runtime execution begins.
    #[test]
    fn bootstrap_populates_session_history_and_metadata() {
        let cwd = env::current_dir().expect("current dir");
        let read_tool: Arc<dyn HostToolAdapter> = Arc::new(StaticTool {
            kind: HostToolKind::Read,
            definition: HostToolDefinition {
                name: "read".to_string(),
                label: "Read".to_string(),
                description: "Read files through the host.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }),
            },
        });

        let config = LynxEmbedConfig::builder(sample_provider())
            .host_tools([read_tool])
            .runtime_metadata(RuntimeMetadata {
                workspace_root: Some(cwd.clone()),
                ..RuntimeMetadata::default()
            })
            .build();
        let transcript = vec![crate::types::HostTranscriptEntry {
            role: crate::types::HostTranscriptRole::User,
            message_id: Some("u1".to_string()),
            tool_call_id: None,
            tool_name: None,
            custom_type: None,
            content: vec![crate::types::HostContentBlock::Text {
                text: "hello".to_string(),
            }],
            is_error: false,
            timestamp_ms: Some(1),
        }];

        let artifacts = bootstrap_turn(&config, &transcript).expect("bootstrap succeeds");

        assert_eq!(artifacts.history.len(), 1);
        assert_eq!(artifacts.history_warnings.len(), 0);
        assert_eq!(artifacts.session.header.cwd, cwd.display().to_string());
        assert_eq!(
            artifacts.session.header.provider.as_deref(),
            Some("openrouter")
        );
        assert_eq!(
            artifacts.session.header.model_id.as_deref(),
            Some("openrouter/auto")
        );
        assert_eq!(
            artifacts.agent_config.stream_options.session_id.as_deref(),
            Some(artifacts.session.header.id.as_str())
        );
        assert_eq!(artifacts.tool_registry.tools().len(), 1);
        let session_messages = artifacts.session.to_messages_for_current_path();
        assert_eq!(session_messages.len(), artifacts.history.len());
        assert!(matches!(
            session_messages.first(),
            Some(pi::sdk::Message::User(message))
                if matches!(&message.content, pi::sdk::UserContent::Text(text) if text == "hello")
        ));
        assert_eq!(
            artifacts.agent_config.system_prompt.as_deref(),
            Some(EMBED_DEFAULT_SYSTEM_PROMPT)
        );
    }

    /// WHY: contradictory tool policy flags should fail during bootstrap so a
    /// mutating or exec-capable surface cannot slip through configuration gaps.
    #[test]
    fn bootstrap_rejects_contradictory_tool_policy() {
        let config = LynxEmbedConfig::builder(sample_provider())
            .tool_policy(ToolPolicy {
                allowed_tools: vec![HostToolKind::Write],
                allow_mutations: false,
                allow_exec: false,
            })
            .build();

        let error = bootstrap_turn(&config, &[]).expect_err("contradictory tool policy");
        assert_eq!(error.kind(), crate::EmbedErrorKind::InvalidConfig);
        assert!(
            error
                .to_string()
                .contains("allows mutating tools without allow_mutations=true")
        );
    }

    fn sample_provider() -> ProviderSelection {
        ProviderSelection {
            provider_id: "open-router".to_string(),
            model_id: "auto".to_string(),
            api_key: Some("test-key".to_string()),
            thinking: None,
            stream_options_override: None,
        }
    }
}

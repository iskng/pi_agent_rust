//! Turn execution for the Lynx embed runtime.

use crate::bootstrap::{BootstrapArtifacts, bootstrap_turn};
use crate::errors::{EmbedError, EmbedErrorKind, Result};
use crate::event_bridge::EventBridge;
use crate::types::{ContinueTurnRequest, TurnRequest, TurnResult, TurnResultMetadata};
use pi::agent::Agent;
use pi::error::Error as PiError;
use pi::model::{AssistantMessage, Message, StopReason, UserContent, UserMessage};
use std::time::{SystemTime, UNIX_EPOCH};

/// Execute one prompt turn using freshly bootstrapped embed artifacts.
pub async fn run_turn(request: TurnRequest) -> Result<TurnResult> {
    if request
        .abort_signal
        .as_ref()
        .is_some_and(pi::sdk::AbortSignal::is_aborted)
    {
        emit_prestart_failure(&request.on_event, EmbedErrorKind::Cancelled);
        return Err(EmbedError::Aborted);
    }

    let artifacts = match bootstrap_turn(&request.config, &request.transcript) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            emit_prestart_failure(&request.on_event, error.kind());
            return Err(error);
        }
    };

    run_turn_with_artifacts(request, artifacts).await
}

/// Execute one prompt turn from preassembled embed artifacts.
pub async fn run_turn_with_artifacts(
    request: TurnRequest,
    artifacts: BootstrapArtifacts,
) -> Result<TurnResult> {
    execute_turn(
        request.config,
        artifacts,
        ExecutionMode::Prompt {
            prompt: request.prompt,
            capture_events: request.capture_events,
        },
        request.on_event,
        request.abort_signal,
    )
    .await
}

/// Continue a reconstructed turn using freshly bootstrapped embed artifacts.
pub async fn continue_turn(request: ContinueTurnRequest) -> Result<TurnResult> {
    if request
        .abort_signal
        .as_ref()
        .is_some_and(pi::sdk::AbortSignal::is_aborted)
    {
        emit_prestart_failure(&request.on_event, EmbedErrorKind::Cancelled);
        return Err(EmbedError::Aborted);
    }

    let artifacts = match bootstrap_turn(&request.config, &request.transcript) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            emit_prestart_failure(&request.on_event, error.kind());
            return Err(error);
        }
    };

    continue_turn_with_artifacts(request, artifacts).await
}

/// Continue a reconstructed turn from preassembled embed artifacts.
pub async fn continue_turn_with_artifacts(
    request: ContinueTurnRequest,
    artifacts: BootstrapArtifacts,
) -> Result<TurnResult> {
    execute_turn(
        request.config,
        artifacts,
        ExecutionMode::Continue {
            capture_events: request.capture_events,
        },
        request.on_event,
        request.abort_signal,
    )
    .await
}

#[derive(Debug)]
enum ExecutionMode {
    Prompt {
        prompt: String,
        capture_events: bool,
    },
    Continue {
        capture_events: bool,
    },
}

async fn execute_turn(
    config: crate::types::LynxEmbedConfig,
    artifacts: BootstrapArtifacts,
    execution_mode: ExecutionMode,
    on_event: Option<std::sync::Arc<dyn Fn(crate::types::EmbedEvent) + Send + Sync>>,
    abort_signal: Option<pi::sdk::AbortSignal>,
) -> Result<TurnResult> {
    let BootstrapArtifacts {
        session,
        tool_registry,
        agent_config,
        provider,
        history,
        history_warnings,
    } = artifacts;

    let provider_id = provider.name().to_string();
    let model_id = provider.model_id().to_string();
    let session_id = session.header.id.clone();

    let bridge = EventBridge::new(on_event, request_capture_events(&execution_mode));
    bridge.emit_turn_started();

    let mut agent = Agent::new(provider, tool_registry, agent_config);
    agent.set_queue_modes(config.queue_mode.steering, config.queue_mode.follow_up);
    agent.replace_messages(history);

    let bridge_for_run = bridge.clone();
    let assistant = match execution_mode {
        ExecutionMode::Prompt { prompt, .. } => {
            let prompt_message = Message::User(UserMessage {
                content: UserContent::Text(prompt),
                timestamp: unix_timestamp_ms(),
            });
            agent
                .run_with_message_with_abort(prompt_message, abort_signal, move |event| {
                    bridge_for_run.handle_agent_event(event);
                })
                .await
        }
        ExecutionMode::Continue { .. } => {
            agent
                .run_continue_with_abort(abort_signal, move |event| {
                    bridge_for_run.handle_agent_event(event);
                })
                .await
        }
    }
    .map_err(|source| finalize_runtime_error(&bridge, &provider_id, &model_id, source))?;

    finalize_success(
        bridge,
        &config,
        &provider_id,
        &model_id,
        &session_id,
        history_warnings,
        assistant,
    )
}

fn finalize_success(
    bridge: EventBridge,
    config: &crate::types::LynxEmbedConfig,
    provider_id: &str,
    model_id: &str,
    session_id: &str,
    history_warnings: Vec<crate::types::HistoryWarning>,
    assistant_message: AssistantMessage,
) -> Result<TurnResult> {
    match assistant_message.stop_reason {
        StopReason::Aborted => {
            tracing::debug!(
                provider = provider_id,
                model_id,
                session_id,
                "Lynx embed turn aborted after execution started"
            );
            bridge.emit_turn_failed(EmbedErrorKind::Cancelled);
        }
        StopReason::Error => {
            let message = assistant_message
                .error_message
                .clone()
                .unwrap_or_else(|| "provider stream failed".to_string());
            let error = EmbedError::provider(
                provider_id.to_string(),
                model_id.to_string(),
                PiError::provider(provider_id.to_string(), message),
            );
            bridge.emit_turn_failed(error.kind());
            return Err(error);
        }
        StopReason::Stop | StopReason::Length | StopReason::ToolUse => {
            bridge.emit_turn_completed();
        }
    }

    let snapshot = bridge.snapshot();
    let aborted = matches!(assistant_message.stop_reason, StopReason::Aborted);
    let had_errors =
        snapshot.had_errors || matches!(assistant_message.stop_reason, StopReason::Error);

    tracing::debug!(
        provider = provider_id,
        model_id,
        session_id,
        stop_reason = ?assistant_message.stop_reason,
        tool_calls_executed = snapshot.tool_calls_executed,
        had_errors,
        aborted,
        "Completed Lynx embed turn"
    );

    Ok(TurnResult {
        stop_reason: Some(assistant_message.stop_reason),
        usage: Some(assistant_message.usage.clone()),
        emitted_events: snapshot.emitted_events,
        history_warnings,
        result_metadata: TurnResultMetadata {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            tool_calls_executed: snapshot.tool_calls_executed,
            had_errors,
            aborted,
            session_mode: config.session_mode.clone(),
        },
        assistant_message,
    })
}

fn finalize_runtime_error(
    bridge: &EventBridge,
    provider_id: &str,
    model_id: &str,
    source: PiError,
) -> EmbedError {
    let error = match source {
        PiError::Aborted => EmbedError::Aborted,
        PiError::Provider { .. } | PiError::Auth(_) | PiError::Api(_) => {
            EmbedError::provider(provider_id.to_string(), model_id.to_string(), source)
        }
        PiError::Tool { tool, message } => EmbedError::tool_failed(tool, message, None),
        PiError::Session(_)
        | PiError::SessionNotFound { .. }
        | PiError::Io(_)
        | PiError::Json(_)
        | PiError::Sqlite(_) => EmbedError::session("runtime::execute_turn", source),
        PiError::Config(message) | PiError::Validation(message) | PiError::Extension(message) => {
            EmbedError::internal(message)
        }
    };

    bridge.emit_turn_failed(error.kind());
    error
}

fn emit_prestart_failure(
    on_event: &Option<std::sync::Arc<dyn Fn(crate::types::EmbedEvent) + Send + Sync>>,
    kind: EmbedErrorKind,
) {
    if let Some(on_event) = on_event {
        on_event(crate::types::EmbedEvent::TurnFailed { error: kind });
    }
}

fn request_capture_events(execution_mode: &ExecutionMode) -> bool {
    match execution_mode {
        ExecutionMode::Prompt { capture_events, .. }
        | ExecutionMode::Continue { capture_events } => *capture_events,
    }
}

fn unix_timestamp_ms() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::stream::{self, Stream};
use pi::error::Error as PiError;
use pi::sdk::{
    AbortHandle, AgentConfig, ContentBlock, Message, Provider, Session, StopReason, StreamEvent,
    StreamOptions, TextContent, ToolRegistry,
};
use pi::sdk::{AssistantMessage, ToolCall, Usage, UserContent, UserMessage};
use pi_lynx_sdk::{
    BootstrapArtifacts, ContinueTurnRequest, EmbedErrorKind, EmbedEvent, HistoryWarningKind,
    HostToolAdapter, HostToolDefinition, HostToolError, HostToolKind, HostToolOutput,
    HostToolRequest, HostTranscriptEntry, HostTranscriptRole, LynxEmbedConfig, ProviderSelection,
    RuntimeMetadata, ToolPolicy, TurnRequest, continue_turn, continue_turn_with_artifacts,
    run_turn, run_turn_with_artifacts,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Debug)]
struct ScriptedProvider {
    steps: Mutex<VecDeque<ProviderStep>>,
}

#[async_trait]
#[allow(clippy::unnecessary_literal_bound)]
impl Provider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn api(&self) -> &str {
        "scripted-api"
    }

    fn model_id(&self) -> &str {
        "scripted-model"
    }

    async fn stream(
        &self,
        context: &pi::provider::Context<'_>,
        _options: &StreamOptions,
    ) -> pi::error::Result<Pin<Box<dyn Stream<Item = pi::error::Result<StreamEvent>> + Send>>> {
        let step = self
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("scripted provider step");
        step.into_stream(context.messages.len(), context.messages.last())
    }
}

#[derive(Debug)]
enum ProviderStep {
    Text {
        expected_messages: usize,
        text: &'static str,
    },
    ToolCalls {
        expected_messages: usize,
        tool_calls: Vec<(&'static str, &'static str, Value)>,
    },
    ToolCall {
        expected_messages: usize,
        tool_call_id: &'static str,
        tool_name: &'static str,
        arguments: Value,
    },
    ExpectToolResultThenText {
        expected_messages: usize,
        expected_tool_call_id: &'static str,
        expected_tool_name: &'static str,
        expected_is_error: bool,
        text: &'static str,
    },
    StreamError {
        expected_messages: usize,
        delta: &'static str,
        error: &'static str,
    },
    PendingAfterStart {
        expected_messages: usize,
    },
}

impl ProviderStep {
    fn into_stream(
        self,
        observed_messages: usize,
        last_message: Option<&Message>,
    ) -> pi::error::Result<Pin<Box<dyn Stream<Item = pi::error::Result<StreamEvent>> + Send>>> {
        match self {
            Self::Text {
                expected_messages,
                text,
            } => {
                assert_eq!(observed_messages, expected_messages);
                Ok(Box::pin(stream::iter(text_events(text))))
            }
            Self::ToolCalls {
                expected_messages,
                tool_calls,
            } => {
                assert_eq!(observed_messages, expected_messages);
                Ok(Box::pin(stream::iter(multi_tool_call_events(tool_calls))))
            }
            Self::ToolCall {
                expected_messages,
                tool_call_id,
                tool_name,
                arguments,
            } => {
                assert_eq!(observed_messages, expected_messages);
                Ok(Box::pin(stream::iter(tool_call_events(
                    tool_call_id,
                    tool_name,
                    arguments,
                ))))
            }
            Self::ExpectToolResultThenText {
                expected_messages,
                expected_tool_call_id,
                expected_tool_name,
                expected_is_error,
                text,
            } => {
                assert_eq!(observed_messages, expected_messages);
                match last_message {
                    Some(Message::ToolResult(tool_result))
                        if tool_result.tool_call_id == expected_tool_call_id
                            && tool_result.tool_name == expected_tool_name
                            && tool_result.is_error == expected_is_error => {}
                    other => panic!("unexpected last message before final response: {other:?}"),
                }
                Ok(Box::pin(stream::iter(text_events(text))))
            }
            Self::StreamError {
                expected_messages,
                delta,
                error,
            } => {
                assert_eq!(observed_messages, expected_messages);
                let mut events = text_prefix_events(delta);
                events.push(Err(PiError::provider("scripted", error)));
                Ok(Box::pin(stream::iter(events)))
            }
            Self::PendingAfterStart { expected_messages } => {
                assert_eq!(observed_messages, expected_messages);
                Ok(Box::pin(
                    stream::iter(vec![Ok(StreamEvent::Start {
                        partial: assistant_message(Vec::new(), StopReason::Stop, None),
                    })])
                    .chain(stream::pending()),
                ))
            }
        }
    }
}

#[derive(Debug)]
struct FailingTool;

#[async_trait]
impl HostToolAdapter for FailingTool {
    fn kind(&self) -> HostToolKind {
        HostToolKind::Read
    }

    fn definition(&self) -> HostToolDefinition {
        HostToolDefinition {
            name: "read".to_string(),
            label: "Read".to_string(),
            description: "Read through the host.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                }
            }),
        }
    }

    async fn execute(
        &self,
        _request: HostToolRequest,
        _on_update: Option<Box<dyn Fn(pi_lynx_sdk::HostToolUpdate) + Send + Sync>>,
    ) -> std::result::Result<HostToolOutput, HostToolError> {
        Err(HostToolError::failed("host read backend unavailable"))
    }
}

#[derive(Debug)]
struct HangingTool;

#[async_trait]
impl HostToolAdapter for HangingTool {
    fn kind(&self) -> HostToolKind {
        HostToolKind::Read
    }

    fn definition(&self) -> HostToolDefinition {
        HostToolDefinition {
            name: "read".to_string(),
            label: "Read".to_string(),
            description: "Read through the host.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                }
            }),
        }
    }

    async fn execute(
        &self,
        _request: HostToolRequest,
        _on_update: Option<Box<dyn Fn(pi_lynx_sdk::HostToolUpdate) + Send + Sync>>,
    ) -> std::result::Result<HostToolOutput, HostToolError> {
        futures::future::pending::<()>().await;
        Ok(HostToolOutput {
            content: Vec::new(),
            details: None,
            is_error: false,
        })
    }
}

#[derive(Debug)]
struct UpdatingHangingExecTool;

#[async_trait]
impl HostToolAdapter for UpdatingHangingExecTool {
    fn kind(&self) -> HostToolKind {
        HostToolKind::Exec
    }

    fn definition(&self) -> HostToolDefinition {
        HostToolDefinition {
            name: "bash".to_string(),
            label: "Bash".to_string(),
            description: "Execute through the host.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            }),
        }
    }

    async fn execute(
        &self,
        _request: HostToolRequest,
        on_update: Option<Box<dyn Fn(pi_lynx_sdk::HostToolUpdate) + Send + Sync>>,
    ) -> std::result::Result<HostToolOutput, HostToolError> {
        if let Some(on_update) = on_update {
            on_update(pi_lynx_sdk::HostToolUpdate {
                content: vec![ContentBlock::Text(TextContent::new(
                    "entered host exec adapter",
                ))],
                details: None,
            });
        }

        futures::future::pending::<()>().await;
        Ok(HostToolOutput {
            content: Vec::new(),
            details: None,
            is_error: false,
        })
    }
}

/// WHY: the embed runtime must produce deterministic host-facing events and a
/// normalized final result for the common no-history prompt path.
#[test]
fn run_turn_with_artifacts_emits_message_events_and_result_metadata() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::Text {
                expected_messages: 1,
                text: "hello from embed",
            }])),
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);

        let result = run_turn_with_artifacts(
            TurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                prompt: "say hello".to_string(),
                on_event: Some(Arc::new(move |event| {
                    events_for_callback
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(event);
                })),
                capture_events: true,
                abort_signal: None,
            },
            bootstrap_artifacts(provider, empty_tool_registry(), Vec::new(), Vec::new()),
        )
        .await
        .expect("turn result");

        assert_eq!(result.result_metadata.provider_id, "scripted");
        assert_eq!(result.result_metadata.model_id, "scripted-model");
        assert_eq!(result.result_metadata.tool_calls_executed, 0);
        assert!(!result.result_metadata.had_errors);
        assert!(!result.result_metadata.aborted);
        assert_eq!(result.stop_reason, Some(StopReason::Stop));
        assert!(matches!(
            result.assistant_message.content.as_slice(),
            [ContentBlock::Text(text)] if text.text == "hello from embed"
        ));

        let emitted = result.emitted_events.expect("captured events");
        assert!(matches!(emitted.first(), Some(EmbedEvent::TurnStarted)));
        assert!(emitted.iter().any(
            |event| matches!(event, EmbedEvent::MessageDelta { text } if text == "hello from embed")
        ));
        assert!(matches!(emitted.last(), Some(EmbedEvent::TurnCompleted)));

        let callback_events = events.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(callback_events.len(), emitted.len());
    });
}

/// WHY: continuation must replay the reconstructed transcript exactly as Pi
/// expects so retry/resume flows do not synthesize a duplicate user message.
#[test]
fn continue_turn_with_artifacts_uses_reconstructed_history() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let history = vec![
            Message::User(UserMessage {
                content: UserContent::Text("inspect repo".to_string()),
                timestamp: 1,
            }),
            Message::assistant(assistant_message(
                vec![ContentBlock::Text(TextContent::new("Working on it."))],
                StopReason::Stop,
                None,
            )),
        ];
        let provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::Text {
                expected_messages: 2,
                text: "continuing from history",
            }])),
        });

        let result = continue_turn_with_artifacts(
            ContinueTurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                on_event: None,
                capture_events: false,
                abort_signal: None,
            },
            bootstrap_artifacts(provider, empty_tool_registry(), history, Vec::new()),
        )
        .await
        .expect("continue result");

        assert_eq!(result.stop_reason, Some(StopReason::Stop));
        assert!(matches!(
            result.assistant_message.content.as_slice(),
            [ContentBlock::Text(text)] if text.text == "continuing from history"
        ));
    });
}

/// WHY: transcript reconstruction warnings must survive the shared execution
/// path so hosts do not lose recoverable diagnostics after bootstrap succeeds.
#[test]
fn run_turn_with_artifacts_preserves_history_warnings() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::Text {
                expected_messages: 1,
                text: "warning surfaced",
            }])),
        });

        let result = run_turn_with_artifacts(
            TurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                prompt: "continue".to_string(),
                on_event: None,
                capture_events: false,
                abort_signal: None,
            },
            bootstrap_artifacts(
                provider,
                empty_tool_registry(),
                Vec::new(),
                vec![pi_lynx_sdk::HistoryWarning {
                    kind: HistoryWarningKind::CustomContentBlockDropped,
                    message_id: Some("c1".to_string()),
                    detail: "dropped custom image".to_string(),
                }],
            ),
        )
        .await
        .expect("turn result");

        assert_eq!(result.history_warnings.len(), 1);
        assert_eq!(
            result.history_warnings[0].kind,
            HistoryWarningKind::CustomContentBlockDropped
        );
        assert_eq!(result.history_warnings[0].message_id.as_deref(), Some("c1"));
    });
}

/// WHY: continuation uses the same runtime assembly path, so it must preserve
/// bootstrap warnings for callers that resume from prevalidated artifacts.
#[test]
fn continue_turn_with_artifacts_preserves_history_warnings() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let history = vec![
            Message::User(UserMessage {
                content: UserContent::Text("inspect repo".to_string()),
                timestamp: 1,
            }),
            Message::assistant(assistant_message(
                vec![ContentBlock::Text(TextContent::new("Working on it."))],
                StopReason::Stop,
                None,
            )),
        ];
        let provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::Text {
                expected_messages: 2,
                text: "warnings preserved on continue",
            }])),
        });

        let result = continue_turn_with_artifacts(
            ContinueTurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                on_event: None,
                capture_events: false,
                abort_signal: None,
            },
            bootstrap_artifacts(
                provider,
                empty_tool_registry(),
                history,
                vec![pi_lynx_sdk::HistoryWarning {
                    kind: HistoryWarningKind::CustomContentBlockDropped,
                    message_id: Some("c1".to_string()),
                    detail: "dropped custom image".to_string(),
                }],
            ),
        )
        .await
        .expect("continue result");

        assert_eq!(result.history_warnings.len(), 1);
        assert_eq!(
            result.history_warnings[0].kind,
            HistoryWarningKind::CustomContentBlockDropped
        );
        assert_eq!(result.history_warnings[0].message_id.as_deref(), Some("c1"));
    });
}

/// WHY: the public `run_turn` entrypoint must short-circuit cleanly when the
/// host aborts before bootstrap so callers get a stable cancellation error.
#[test]
fn run_turn_rejects_prestart_abort() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let (abort_handle, abort_signal) = AbortHandle::new();
        abort_handle.abort();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);

        let error = run_turn(TurnRequest {
            config: sample_config(),
            transcript: Vec::new(),
            prompt: "ignored".to_string(),
            on_event: Some(Arc::new(move |event| {
                events_for_callback
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event);
            })),
            capture_events: false,
            abort_signal: Some(abort_signal),
        })
        .await
        .expect_err("prestart abort must fail");

        assert_eq!(error.kind(), EmbedErrorKind::Cancelled);
        let events = events.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events.first(),
            Some(EmbedEvent::TurnFailed {
                error: EmbedErrorKind::Cancelled
            })
        ));
    });
}

/// WHY: bootstrap must reject persisted partial turns with unresolved tool
/// calls so continuation never sends an impossible assistant/tool sequence back
/// to the provider.
#[test]
fn continue_turn_rejects_unresolved_tool_call_transcript_before_execution() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);

        let error = continue_turn(ContinueTurnRequest {
            config: sample_config(),
            transcript: vec![assistant_tool_call_transcript_entry(
                "a1", "call_1", "read", 1,
            )],
            on_event: Some(Arc::new(move |event| {
                events_for_callback
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event);
            })),
            capture_events: false,
            abort_signal: None,
        })
        .await
        .expect_err("unresolved tool transcript must fail before execution");

        assert_eq!(error.kind(), EmbedErrorKind::InvalidTranscript);
        assert!(
            error
                .to_string()
                .contains("unresolved assistant tool_call_id")
        );

        let events = events.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events.first(),
            Some(EmbedEvent::TurnFailed {
                error: EmbedErrorKind::InvalidTranscript
            })
        ));
    });
}

/// WHY: provider stream failures after execution starts must surface a stable
/// runtime error and emit `TurnFailed` instead of pretending the turn ended
/// successfully.
#[test]
fn run_turn_with_artifacts_converts_provider_stream_failure_into_error() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::StreamError {
                expected_messages: 1,
                delta: "partial",
                error: "stream exploded",
            }])),
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);

        let error = run_turn_with_artifacts(
            TurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                prompt: "break the stream".to_string(),
                on_event: Some(Arc::new(move |event| {
                    events_for_callback
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(event);
                })),
                capture_events: false,
                abort_signal: None,
            },
            bootstrap_artifacts(provider, empty_tool_registry(), Vec::new(), Vec::new()),
        )
        .await
        .expect_err("provider stream failure must error");

        assert_eq!(error.kind(), EmbedErrorKind::ProviderStreamFailed);
        assert!(error.to_string().contains("stream exploded"));
        assert!(
            events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|event| matches!(
                    event,
                    EmbedEvent::TurnFailed {
                        error: EmbedErrorKind::ProviderStreamFailed
                    }
                ))
        );
    });
}

/// WHY: tool failures that remain representable as tool results must not crash
/// the turn, and host aborts during streaming or tool execution must still
/// yield normalized aborted results for partial-turn recovery.
#[test]
fn runtime_handles_tool_failures_and_abort_paths() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let tool_registry = pi_lynx_sdk::build_tool_registry(
            &ToolPolicy::default(),
            &RuntimeMetadata::default(),
            &[Arc::new(FailingTool) as Arc<dyn HostToolAdapter>],
        )
        .expect("tool registry");
        let tool_provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([
                ProviderStep::ToolCall {
                    expected_messages: 1,
                    tool_call_id: "call_1",
                    tool_name: "read",
                    arguments: json!({ "path": "README.md" }),
                },
                ProviderStep::ExpectToolResultThenText {
                    expected_messages: 3,
                    expected_tool_call_id: "call_1",
                    expected_tool_name: "read",
                    expected_is_error: true,
                    text: "tool error observed",
                },
            ])),
        });

        let tool_result = run_turn_with_artifacts(
            TurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                prompt: "read the file".to_string(),
                on_event: None,
                capture_events: false,
                abort_signal: None,
            },
            bootstrap_artifacts(tool_provider, tool_registry, Vec::new(), Vec::new()),
        )
        .await
        .expect("tool failure stays in-band");

        assert!(tool_result.result_metadata.had_errors);
        assert_eq!(tool_result.result_metadata.tool_calls_executed, 1);
        assert!(matches!(
            tool_result.assistant_message.content.as_slice(),
            [ContentBlock::Text(text)] if text.text == "tool error observed"
        ));

        let (stream_abort_handle, stream_abort_signal) = AbortHandle::new();
        let stream_provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::PendingAfterStart {
                expected_messages: 1,
            }])),
        });
        let stream_result = run_turn_with_artifacts(
            TurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                prompt: "abort me".to_string(),
                on_event: Some(Arc::new(move |event| {
                    if matches!(event, EmbedEvent::ProviderEvent { .. }) {
                        stream_abort_handle.abort();
                    }
                })),
                capture_events: true,
                abort_signal: Some(stream_abort_signal),
            },
            bootstrap_artifacts(
                stream_provider,
                empty_tool_registry(),
                Vec::new(),
                Vec::new(),
            ),
        )
        .await
        .expect("abort during provider stream returns partial result");

        assert_eq!(stream_result.stop_reason, Some(StopReason::Aborted));
        assert!(stream_result.result_metadata.aborted);
        assert!(matches!(
            stream_result
                .emitted_events
                .as_deref()
                .and_then(|events| events.last()),
            Some(EmbedEvent::TurnFailed {
                error: EmbedErrorKind::Cancelled
            })
        ));

        let (tool_abort_handle, tool_abort_signal) = AbortHandle::new();
        let hanging_registry = pi_lynx_sdk::build_tool_registry(
            &ToolPolicy::default(),
            &RuntimeMetadata::default(),
            &[Arc::new(HangingTool) as Arc<dyn HostToolAdapter>],
        )
        .expect("hanging tool registry");
        let hanging_provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::ToolCall {
                expected_messages: 1,
                tool_call_id: "call_2",
                tool_name: "read",
                arguments: json!({ "path": "README.md" }),
            }])),
        });
        let tool_abort_result = run_turn_with_artifacts(
            TurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                prompt: "abort the tool".to_string(),
                on_event: Some(Arc::new(move |event| {
                    if matches!(event, EmbedEvent::ToolStarted { .. }) {
                        tool_abort_handle.abort();
                    }
                })),
                capture_events: false,
                abort_signal: Some(tool_abort_signal),
            },
            bootstrap_artifacts(hanging_provider, hanging_registry, Vec::new(), Vec::new()),
        )
        .await
        .expect("abort during tool execution returns partial result");

        assert_eq!(tool_abort_result.stop_reason, Some(StopReason::Aborted));
        assert!(tool_abort_result.result_metadata.aborted);
        assert_eq!(tool_abort_result.result_metadata.tool_calls_executed, 1);
        assert!(tool_abort_result.result_metadata.had_errors);
        assert!(tool_abort_result.emitted_events.is_none());
    });
}

/// WHY: interrupted multi-tool batches must count only host tools that
/// actually entered adapter execution, excluding later synthetic abort ends.
#[test]
fn runtime_counts_only_started_tools_in_aborted_multi_tool_batches() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let (abort_handle, abort_signal) = AbortHandle::new();
        let tool_registry = pi_lynx_sdk::build_tool_registry(
            &ToolPolicy {
                allowed_tools: vec![HostToolKind::Exec],
                allow_mutations: false,
                allow_exec: true,
            },
            &RuntimeMetadata::default(),
            &[Arc::new(UpdatingHangingExecTool) as Arc<dyn HostToolAdapter>],
        )
        .expect("tool registry");
        let provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::ToolCalls {
                expected_messages: 1,
                tool_calls: vec![
                    ("call_1", "bash", json!({ "command": "sleep 10" })),
                    ("call_2", "bash", json!({ "command": "echo never" })),
                ],
            }])),
        });

        let result = run_turn_with_artifacts(
            TurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                prompt: "run both commands".to_string(),
                on_event: Some(Arc::new(move |event| {
                    if matches!(
                        event,
                        EmbedEvent::ToolUpdate { tool_call_id, .. } if tool_call_id == "call_1"
                    ) {
                        abort_handle.abort();
                    }
                })),
                capture_events: true,
                abort_signal: Some(abort_signal),
            },
            bootstrap_artifacts(provider, tool_registry, Vec::new(), Vec::new()),
        )
        .await
        .expect("abort during multi-tool execution returns partial result");

        assert_eq!(result.stop_reason, Some(StopReason::Aborted));
        assert!(result.result_metadata.aborted);
        assert_eq!(result.result_metadata.tool_calls_executed, 1);
        assert!(result.result_metadata.had_errors);
        let emitted_events = result.emitted_events.expect("captured events");
        assert!(emitted_events.iter().any(|event| matches!(
            event,
            EmbedEvent::ToolStarted { tool_call_id, .. } if tool_call_id == "call_1"
        )));
        assert!(!emitted_events.iter().any(|event| matches!(
            event,
            EmbedEvent::ToolStarted { tool_call_id, .. } if tool_call_id == "call_2"
        )));
        assert!(emitted_events.iter().any(|event| matches!(
            event,
            EmbedEvent::ToolCompleted {
                tool_call_id,
                executed: true,
                ..
            } if tool_call_id == "call_1"
        )));
        assert!(emitted_events.iter().any(|event| matches!(
            event,
            EmbedEvent::ToolCompleted {
                tool_call_id,
                executed: false,
                ..
            } if tool_call_id == "call_2"
        )));
    });
}

/// WHY: hosts may consume stream callbacks without retaining a second in-memory
/// copy of the entire event stream for the completed turn result.
#[test]
fn run_turn_with_artifacts_skips_event_capture_when_not_requested() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let provider = Arc::new(ScriptedProvider {
            steps: Mutex::new(VecDeque::from([ProviderStep::Text {
                expected_messages: 1,
                text: "hello without capture",
            }])),
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);

        let result = run_turn_with_artifacts(
            TurnRequest {
                config: sample_config(),
                transcript: Vec::new(),
                prompt: "say hello".to_string(),
                on_event: Some(Arc::new(move |event| {
                    events_for_callback
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(event);
                })),
                capture_events: false,
                abort_signal: None,
            },
            bootstrap_artifacts(provider, empty_tool_registry(), Vec::new(), Vec::new()),
        )
        .await
        .expect("turn result");

        assert!(result.emitted_events.is_none());

        let callback_events = events.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(!callback_events.is_empty());
        assert!(matches!(
            callback_events.first(),
            Some(EmbedEvent::TurnStarted)
        ));
        assert!(matches!(
            callback_events.last(),
            Some(EmbedEvent::TurnCompleted)
        ));
    });
}

/// WHY: multi-tool continuation must fail before execution when persisted tool
/// results do not match the replay order Pi uses inside the agent loop.
#[test]
fn continue_turn_rejects_out_of_order_multi_tool_replay_before_execution() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");

    runtime.block_on(async {
        let error = continue_turn(ContinueTurnRequest {
            config: sample_config(),
            transcript: vec![
                HostTranscriptEntry {
                    role: HostTranscriptRole::Assistant,
                    message_id: Some("a1".to_string()),
                    tool_call_id: None,
                    tool_name: None,
                    custom_type: None,
                    content: vec![
                        pi_lynx_sdk::HostContentBlock::ToolCall {
                            tool_call_id: "call_1".to_string(),
                            tool_name: "read".to_string(),
                            arguments: json!({ "path": "README.md" }),
                        },
                        pi_lynx_sdk::HostContentBlock::ToolCall {
                            tool_call_id: "call_2".to_string(),
                            tool_name: "search".to_string(),
                            arguments: json!({ "pattern": "lynx" }),
                        },
                    ],
                    is_error: false,
                    timestamp_ms: Some(1),
                },
                HostTranscriptEntry {
                    role: HostTranscriptRole::ToolResult,
                    message_id: Some("t2".to_string()),
                    tool_call_id: Some("call_2".to_string()),
                    tool_name: Some("search".to_string()),
                    custom_type: None,
                    content: vec![pi_lynx_sdk::HostContentBlock::Text {
                        text: "search output".to_string(),
                    }],
                    is_error: false,
                    timestamp_ms: Some(2),
                },
            ],
            on_event: None,
            capture_events: false,
            abort_signal: None,
        })
        .await
        .expect_err("out-of-order transcript must fail before execution");

        assert_eq!(error.kind(), EmbedErrorKind::InvalidTranscript);
        assert!(error.to_string().contains("expected 'call_1'"));
    });
}

fn sample_config() -> LynxEmbedConfig {
    LynxEmbedConfig::builder(ProviderSelection {
        provider_id: "openrouter".to_string(),
        model_id: "auto".to_string(),
        api_key: None,
        thinking: None,
        stream_options_override: None,
    })
    .build()
}

fn bootstrap_artifacts(
    provider: Arc<dyn Provider>,
    tool_registry: ToolRegistry,
    history: Vec<Message>,
    history_warnings: Vec<pi_lynx_sdk::HistoryWarning>,
) -> BootstrapArtifacts {
    let mut session = Session::in_memory();
    session.set_model_header(
        Some(provider.name().to_string()),
        Some(provider.model_id().to_string()),
        None,
    );
    for message in &history {
        session.append_model_message(message.clone());
    }

    BootstrapArtifacts {
        agent_config: AgentConfig {
            system_prompt: Some("embedded test".to_string()),
            max_tool_iterations: 8,
            stream_options: StreamOptions {
                session_id: Some(session.header.id.clone()),
                ..StreamOptions::default()
            },
            block_images: false,
        },
        session,
        tool_registry,
        provider,
        history,
        history_warnings,
    }
}

fn empty_tool_registry() -> ToolRegistry {
    ToolRegistry::from_tools(Vec::new())
}

fn assistant_message(
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    error_message: Option<&str>,
) -> AssistantMessage {
    AssistantMessage {
        content,
        api: "scripted-api".to_string(),
        provider: "scripted".to_string(),
        model: "scripted-model".to_string(),
        usage: Usage::default(),
        stop_reason,
        error_message: error_message.map(str::to_string),
        timestamp: 1,
    }
}

fn assistant_tool_call_transcript_entry(
    message_id: &str,
    tool_call_id: &str,
    tool_name: &str,
    timestamp_ms: i64,
) -> HostTranscriptEntry {
    HostTranscriptEntry {
        role: HostTranscriptRole::Assistant,
        message_id: Some(message_id.to_string()),
        tool_call_id: None,
        tool_name: None,
        custom_type: None,
        content: vec![pi_lynx_sdk::HostContentBlock::ToolCall {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: json!({ "path": "README.md" }),
        }],
        is_error: false,
        timestamp_ms: Some(timestamp_ms),
    }
}

fn text_prefix_events(delta: &str) -> Vec<pi::error::Result<StreamEvent>> {
    vec![
        Ok(StreamEvent::Start {
            partial: assistant_message(Vec::new(), StopReason::Stop, None),
        }),
        Ok(StreamEvent::TextStart { content_index: 0 }),
        Ok(StreamEvent::TextDelta {
            content_index: 0,
            delta: delta.to_string(),
        }),
    ]
}

fn text_events(text: &str) -> Vec<pi::error::Result<StreamEvent>> {
    let mut events = text_prefix_events(text);
    let message = assistant_message(
        vec![ContentBlock::Text(TextContent::new(text))],
        StopReason::Stop,
        None,
    );
    events.extend([
        Ok(StreamEvent::TextEnd {
            content_index: 0,
            content: text.to_string(),
        }),
        Ok(StreamEvent::Done {
            reason: StopReason::Stop,
            message,
        }),
    ]);
    events
}

fn tool_call_events(
    tool_call_id: &str,
    tool_name: &str,
    arguments: Value,
) -> Vec<pi::error::Result<StreamEvent>> {
    let tool_call = ToolCall {
        id: tool_call_id.to_string(),
        name: tool_name.to_string(),
        arguments,
        thought_signature: None,
    };
    let message = assistant_message(
        vec![ContentBlock::ToolCall(tool_call.clone())],
        StopReason::ToolUse,
        None,
    );

    vec![
        Ok(StreamEvent::Start {
            partial: assistant_message(Vec::new(), StopReason::ToolUse, None),
        }),
        Ok(StreamEvent::ToolCallStart { content_index: 0 }),
        Ok(StreamEvent::ToolCallEnd {
            content_index: 0,
            tool_call,
        }),
        Ok(StreamEvent::Done {
            reason: StopReason::ToolUse,
            message,
        }),
    ]
}

fn multi_tool_call_events(
    tool_calls: Vec<(&str, &str, Value)>,
) -> Vec<pi::error::Result<StreamEvent>> {
    let tool_calls = tool_calls
        .into_iter()
        .map(|(tool_call_id, tool_name, arguments)| ToolCall {
            id: tool_call_id.to_string(),
            name: tool_name.to_string(),
            arguments,
            thought_signature: None,
        })
        .collect::<Vec<_>>();
    let message = assistant_message(
        tool_calls
            .iter()
            .cloned()
            .map(ContentBlock::ToolCall)
            .collect(),
        StopReason::ToolUse,
        None,
    );

    let mut events = vec![Ok(StreamEvent::Start {
        partial: assistant_message(Vec::new(), StopReason::ToolUse, None),
    })];
    for (content_index, tool_call) in tool_calls.into_iter().enumerate() {
        events.push(Ok(StreamEvent::ToolCallStart { content_index }));
        events.push(Ok(StreamEvent::ToolCallEnd {
            content_index,
            tool_call,
        }));
    }
    events.push(Ok(StreamEvent::Done {
        reason: StopReason::ToolUse,
        message,
    }));
    events
}

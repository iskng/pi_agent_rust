//! Deterministic translation from Pi agent events into embed-facing events.

use crate::types::{EmbedEvent, ToolUpdatePayload};
use pi::agent::AgentEvent;
use pi::model::{AssistantMessageEvent, Message, StopReason, StreamEvent};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

type EventCallback = Arc<dyn Fn(EmbedEvent) + Send + Sync>;

/// Snapshot of embed event emission collected during one turn.
#[derive(Debug, Clone)]
pub(crate) struct EventBridgeSnapshot {
    pub emitted_events: Vec<EmbedEvent>,
    pub tool_calls_executed: usize,
    pub had_errors: bool,
}

#[derive(Clone)]
pub(crate) struct EventBridge {
    inner: Arc<Mutex<EventBridgeState>>,
}

impl EventBridge {
    pub(crate) fn new(callback: Option<EventCallback>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventBridgeState {
                callback,
                emitted_events: Vec::new(),
                tool_calls_executed: 0,
                had_errors: false,
                terminal_emitted: false,
            })),
        }
    }

    pub(crate) fn emit_turn_started(&self) {
        self.emit(EmbedEvent::TurnStarted);
    }

    pub(crate) fn emit_turn_completed(&self) {
        let should_emit = {
            let mut state = self.lock_state();
            if state.terminal_emitted {
                false
            } else {
                state.terminal_emitted = true;
                true
            }
        };

        if should_emit {
            self.emit(EmbedEvent::TurnCompleted);
        }
    }

    pub(crate) fn emit_turn_failed(&self, error: crate::errors::EmbedErrorKind) {
        let should_emit = {
            let mut state = self.lock_state();
            state.had_errors = true;
            if state.terminal_emitted {
                false
            } else {
                state.terminal_emitted = true;
                true
            }
        };

        if should_emit {
            self.emit(EmbedEvent::TurnFailed { error });
        }
    }

    pub(crate) fn handle_agent_event(&self, event: AgentEvent) {
        match event {
            AgentEvent::MessageUpdate {
                assistant_message_event,
                ..
            } => {
                if let Some(provider_event) =
                    provider_event_from_assistant_event(&assistant_message_event)
                {
                    self.emit(EmbedEvent::ProviderEvent {
                        event: provider_event,
                    });
                }

                match assistant_message_event {
                    AssistantMessageEvent::TextDelta { delta, .. } => {
                        self.emit(EmbedEvent::MessageDelta { text: delta });
                    }
                    AssistantMessageEvent::Error { reason, .. } => {
                        if matches!(reason, StopReason::Error | StopReason::Aborted) {
                            self.lock_state().had_errors = true;
                        }
                    }
                    AssistantMessageEvent::Done { reason, .. } => {
                        if matches!(reason, StopReason::Error | StopReason::Aborted) {
                            self.lock_state().had_errors = true;
                        }
                    }
                    AssistantMessageEvent::Start { .. }
                    | AssistantMessageEvent::TextStart { .. }
                    | AssistantMessageEvent::TextEnd { .. }
                    | AssistantMessageEvent::ThinkingStart { .. }
                    | AssistantMessageEvent::ThinkingDelta { .. }
                    | AssistantMessageEvent::ThinkingEnd { .. }
                    | AssistantMessageEvent::ToolCallStart { .. }
                    | AssistantMessageEvent::ToolCallDelta { .. }
                    | AssistantMessageEvent::ToolCallEnd { .. } => {}
                }
            }
            AgentEvent::MessageEnd { message } => {
                if should_emit_completed_message(&message) {
                    self.emit(EmbedEvent::MessageCompleted { message });
                }
            }
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                self.emit(EmbedEvent::ToolStarted {
                    tool_call_id,
                    tool_name,
                    args,
                });
            }
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial_result,
                ..
            } => {
                self.emit(EmbedEvent::ToolUpdate {
                    tool_call_id,
                    update: ToolUpdatePayload {
                        content: partial_result.content,
                        details: partial_result.details,
                    },
                });
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                is_error,
                ..
            } => {
                let mut state = self.lock_state();
                state.tool_calls_executed += 1;
                state.had_errors |= is_error;
                drop(state);
                self.emit(EmbedEvent::ToolCompleted {
                    tool_call_id,
                    tool_name,
                    is_error,
                });
            }
            AgentEvent::AgentStart { .. }
            | AgentEvent::AgentEnd { .. }
            | AgentEvent::TurnStart { .. }
            | AgentEvent::TurnEnd { .. }
            | AgentEvent::MessageStart { .. }
            | AgentEvent::AutoCompactionStart { .. }
            | AgentEvent::AutoCompactionEnd { .. }
            | AgentEvent::AutoRetryStart { .. }
            | AgentEvent::AutoRetryEnd { .. }
            | AgentEvent::ExtensionError { .. } => {}
        }
    }

    pub(crate) fn snapshot(&self) -> EventBridgeSnapshot {
        let state = self.lock_state();
        EventBridgeSnapshot {
            emitted_events: state.emitted_events.clone(),
            tool_calls_executed: state.tool_calls_executed,
            had_errors: state.had_errors,
        }
    }

    fn emit(&self, event: EmbedEvent) {
        let callback = {
            let mut state = self.lock_state();
            state.emitted_events.push(event.clone());
            state.callback.clone()
        };

        if let Some(callback) = callback {
            callback(event);
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, EventBridgeState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
struct EventBridgeState {
    callback: Option<EventCallback>,
    emitted_events: Vec<EmbedEvent>,
    tool_calls_executed: usize,
    had_errors: bool,
    terminal_emitted: bool,
}

fn should_emit_completed_message(message: &Message) -> bool {
    !matches!(message, Message::User(_))
}

fn provider_event_from_assistant_event(event: &AssistantMessageEvent) -> Option<StreamEvent> {
    let provider_event = match event {
        AssistantMessageEvent::Start { partial } => StreamEvent::Start {
            partial: partial.as_ref().clone(),
        },
        AssistantMessageEvent::TextStart { content_index, .. } => StreamEvent::TextStart {
            content_index: *content_index,
        },
        AssistantMessageEvent::TextDelta {
            content_index,
            delta,
            ..
        } => StreamEvent::TextDelta {
            content_index: *content_index,
            delta: delta.clone(),
        },
        AssistantMessageEvent::TextEnd {
            content_index,
            content,
            ..
        } => StreamEvent::TextEnd {
            content_index: *content_index,
            content: content.clone(),
        },
        AssistantMessageEvent::ThinkingStart { content_index, .. } => StreamEvent::ThinkingStart {
            content_index: *content_index,
        },
        AssistantMessageEvent::ThinkingDelta {
            content_index,
            delta,
            ..
        } => StreamEvent::ThinkingDelta {
            content_index: *content_index,
            delta: delta.clone(),
        },
        AssistantMessageEvent::ThinkingEnd {
            content_index,
            content,
            ..
        } => StreamEvent::ThinkingEnd {
            content_index: *content_index,
            content: content.clone(),
        },
        AssistantMessageEvent::ToolCallStart { content_index, .. } => StreamEvent::ToolCallStart {
            content_index: *content_index,
        },
        AssistantMessageEvent::ToolCallDelta {
            content_index,
            delta,
            ..
        } => StreamEvent::ToolCallDelta {
            content_index: *content_index,
            delta: delta.clone(),
        },
        AssistantMessageEvent::ToolCallEnd {
            content_index,
            tool_call,
            ..
        } => StreamEvent::ToolCallEnd {
            content_index: *content_index,
            tool_call: tool_call.clone(),
        },
        AssistantMessageEvent::Done { reason, message } => StreamEvent::Done {
            reason: *reason,
            message: message.as_ref().clone(),
        },
        AssistantMessageEvent::Error { reason, error } => StreamEvent::Error {
            reason: *reason,
            error: error.as_ref().clone(),
        },
    };
    Some(provider_event)
}

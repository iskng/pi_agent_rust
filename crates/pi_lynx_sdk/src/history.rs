//! Transcript reconstruction helpers for embed mode.

use crate::errors::{EmbedError, Result};
use crate::types::{
    HistoryConversionResult, HistoryWarning, HistoryWarningKind, HostContentBlock,
    HostTranscriptEntry, HostTranscriptRole,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use pi::sdk::{
    AssistantMessage, ContentBlock, CustomMessage, ImageContent, Message, StopReason, TextContent,
    ThinkingContent, ToolCall, ToolResultMessage, Usage, UserContent, UserMessage,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Reconstruct Pi message history from host-owned transcript entries.
pub fn reconstruct_history(transcript: &[HostTranscriptEntry]) -> Result<HistoryConversionResult> {
    let mut messages = Vec::with_capacity(transcript.len());
    let mut warnings = Vec::new();
    let mut active_tool_calls = BTreeMap::<String, String>::new();
    let mut pending_tool_calls = VecDeque::<(String, String)>::new();
    let mut completed_tool_calls = BTreeSet::<String>::new();

    for entry in transcript {
        match entry.role {
            HostTranscriptRole::User => {
                ensure_no_pending_tool_calls(entry, &pending_tool_calls)?;
                clear_resolved_tool_batch(
                    &pending_tool_calls,
                    &mut active_tool_calls,
                    &mut completed_tool_calls,
                );
                let blocks = convert_blocks(entry, TranscriptRole::User, &mut warnings)?;
                let content = match blocks.as_slice() {
                    [ContentBlock::Text(text)] => UserContent::Text(text.text.clone()),
                    _ => UserContent::Blocks(blocks),
                };
                messages.push(Message::User(UserMessage {
                    content,
                    timestamp: entry.timestamp_ms.unwrap_or_default(),
                }));
            }
            HostTranscriptRole::Assistant => {
                ensure_no_pending_tool_calls(entry, &pending_tool_calls)?;
                clear_resolved_tool_batch(
                    &pending_tool_calls,
                    &mut active_tool_calls,
                    &mut completed_tool_calls,
                );
                let content = convert_blocks(entry, TranscriptRole::Assistant, &mut warnings)?;
                for block in &content {
                    if let ContentBlock::ToolCall(tool_call) = block {
                        if active_tool_calls
                            .insert(tool_call.id.clone(), tool_call.name.clone())
                            .is_some()
                        {
                            return Err(transcript_error(
                                entry,
                                format!(
                                    "duplicate tool_call_id '{}' in assistant history",
                                    tool_call.id
                                ),
                            ));
                        }
                        pending_tool_calls
                            .push_back((tool_call.id.clone(), tool_call.name.clone()));
                    }
                }

                let stop_reason = if content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::ToolCall(_)))
                {
                    StopReason::ToolUse
                } else if entry.is_error {
                    StopReason::Error
                } else {
                    StopReason::Stop
                };

                messages.push(Message::assistant(AssistantMessage {
                    content,
                    api: String::new(),
                    provider: String::new(),
                    model: String::new(),
                    usage: Usage::default(),
                    stop_reason,
                    error_message: entry.is_error.then_some(
                        "host transcript marked reconstructed assistant message as error"
                            .to_string(),
                    ),
                    timestamp: entry.timestamp_ms.unwrap_or_default(),
                }));
            }
            HostTranscriptRole::ToolResult => {
                let tool_call_id = entry.tool_call_id.clone().ok_or_else(|| {
                    transcript_error(entry, "tool result entry is missing tool_call_id")
                })?;
                let tool_name = entry.tool_name.clone().ok_or_else(|| {
                    transcript_error(entry, "tool result entry is missing tool_name")
                })?;
                let Some(expected_name) = active_tool_calls.get(&tool_call_id) else {
                    return Err(transcript_error(
                        entry,
                        if completed_tool_calls.contains(&tool_call_id) {
                            format!("duplicate tool result for tool_call_id '{tool_call_id}'")
                        } else {
                            format!("tool result references unknown tool_call_id '{tool_call_id}'")
                        },
                    ));
                };
                let Some((pending_tool_call_id, pending_name)) =
                    pending_tool_calls.front().cloned()
                else {
                    let message = if completed_tool_calls.contains(&tool_call_id) {
                        format!("duplicate tool result for tool_call_id '{tool_call_id}'")
                    } else {
                        format!(
                            "tool result for tool_call_id '{tool_call_id}' is out of order; \
all assistant tool calls must be resolved before the next non-tool transcript entry"
                        )
                    };
                    return Err(transcript_error(entry, message));
                };
                if pending_tool_call_id != tool_call_id {
                    return Err(transcript_error(
                        entry,
                        format!(
                            "tool result for tool_call_id '{tool_call_id}' is out of order; \
expected '{pending_tool_call_id}' next to match the assistant tool-call replay order"
                        ),
                    ));
                }
                if expected_name != &tool_name {
                    return Err(transcript_error(
                        entry,
                        format!(
                            "tool result tool_name '{}' does not match assistant tool call '{}'",
                            tool_name, expected_name
                        ),
                    ));
                }
                if pending_name != tool_name {
                    return Err(transcript_error(
                        entry,
                        format!(
                            "tool result tool_name '{}' does not match pending assistant tool call '{}'",
                            tool_name, pending_name
                        ),
                    ));
                }
                pending_tool_calls.pop_front();
                if !completed_tool_calls.insert(tool_call_id.clone()) {
                    return Err(transcript_error(
                        entry,
                        format!("duplicate tool result for tool_call_id '{tool_call_id}'"),
                    ));
                }

                let content = convert_blocks(entry, TranscriptRole::ToolResult, &mut warnings)?;
                messages.push(Message::tool_result(ToolResultMessage {
                    tool_call_id,
                    tool_name,
                    content,
                    details: None,
                    is_error: entry.is_error,
                    timestamp: entry.timestamp_ms.unwrap_or_default(),
                }));
            }
            HostTranscriptRole::Custom => {
                ensure_no_pending_tool_calls(entry, &pending_tool_calls)?;
                let mut text_fragments = Vec::new();
                for block in &entry.content {
                    match block {
                        HostContentBlock::Text { text } => text_fragments.push(text.clone()),
                        HostContentBlock::Image { .. }
                        | HostContentBlock::Thinking { .. }
                        | HostContentBlock::ToolCall { .. } => {
                            warnings.push(HistoryWarning {
                                kind: HistoryWarningKind::CustomContentBlockDropped,
                                message_id: entry.message_id.clone(),
                                detail: format!(
                                    "Dropped non-text custom content block while reconstructing message '{}'",
                                    entry.message_id.as_deref().unwrap_or("<unknown>")
                                ),
                            });
                        }
                    }
                }

                messages.push(Message::Custom(CustomMessage {
                    content: text_fragments.join("\n"),
                    custom_type: entry
                        .custom_type
                        .clone()
                        .unwrap_or_else(|| "host_custom".to_string()),
                    display: false,
                    details: None,
                    timestamp: entry.timestamp_ms.unwrap_or_default(),
                }));
            }
        }
    }

    if !pending_tool_calls.is_empty() {
        let unresolved = pending_tool_calls
            .iter()
            .map(|(tool_call_id, _)| tool_call_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(EmbedError::transcript(format!(
            "transcript ended with unresolved assistant tool_call_id(s): {unresolved}"
        )));
    }

    Ok(HistoryConversionResult { messages, warnings })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptRole {
    User,
    Assistant,
    ToolResult,
}

fn convert_blocks(
    entry: &HostTranscriptEntry,
    role: TranscriptRole,
    warnings: &mut Vec<HistoryWarning>,
) -> Result<Vec<ContentBlock>> {
    let mut blocks = Vec::with_capacity(entry.content.len());
    for block in &entry.content {
        match block {
            HostContentBlock::Text { text } => {
                blocks.push(ContentBlock::Text(TextContent::new(text)))
            }
            HostContentBlock::Image { mime_type, data } => {
                blocks.push(ContentBlock::Image(ImageContent {
                    data: BASE64_STANDARD.encode(data),
                    mime_type: mime_type.clone(),
                }))
            }
            HostContentBlock::Thinking { text } => {
                blocks.push(ContentBlock::Thinking(ThinkingContent {
                    thinking: text.clone(),
                    thinking_signature: None,
                }))
            }
            HostContentBlock::ToolCall {
                tool_call_id,
                tool_name,
                arguments,
            } => {
                if role != TranscriptRole::Assistant {
                    return Err(transcript_error(
                        entry,
                        "tool call blocks are only valid inside assistant transcript entries",
                    ));
                }

                let tool_call_id = tool_call_id.trim();
                if tool_call_id.is_empty() {
                    return Err(transcript_error(
                        entry,
                        "assistant tool call block is missing tool_call_id",
                    ));
                }

                let tool_name = tool_name.trim();
                if tool_name.is_empty() {
                    return Err(transcript_error(
                        entry,
                        "assistant tool call block is missing tool_name",
                    ));
                }

                blocks.push(ContentBlock::ToolCall(ToolCall {
                    id: tool_call_id.to_string(),
                    name: tool_name.to_string(),
                    arguments: arguments.clone(),
                    thought_signature: None,
                }));
            }
        }
    }

    if blocks.is_empty() && role != TranscriptRole::ToolResult {
        warnings.push(HistoryWarning {
            kind: HistoryWarningKind::CustomContentBlockDropped,
            message_id: entry.message_id.clone(),
            detail: format!(
                "Transcript entry '{}' reconstructed to an empty content list",
                entry.message_id.as_deref().unwrap_or("<unknown>")
            ),
        });
    }

    Ok(blocks)
}

fn transcript_error(entry: &HostTranscriptEntry, message: impl Into<String>) -> EmbedError {
    let message = message.into();
    let context = entry.message_id.as_deref().map_or_else(
        || format!("role {:?}: {message}", entry.role),
        |message_id| format!("message_id {message_id}: {message}"),
    );
    EmbedError::transcript(context)
}

fn clear_resolved_tool_batch(
    pending_tool_calls: &VecDeque<(String, String)>,
    active_tool_calls: &mut BTreeMap<String, String>,
    completed_tool_calls: &mut BTreeSet<String>,
) {
    if pending_tool_calls.is_empty() {
        active_tool_calls.clear();
        completed_tool_calls.clear();
    }
}

fn ensure_no_pending_tool_calls(
    entry: &HostTranscriptEntry,
    pending_tool_calls: &VecDeque<(String, String)>,
) -> Result<()> {
    if pending_tool_calls.is_empty() {
        return Ok(());
    }

    let unresolved = pending_tool_calls
        .iter()
        .map(|(tool_call_id, _)| tool_call_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(transcript_error(
        entry,
        format!(
            "encountered {:?} transcript entry while assistant tool_call_id(s) remain unresolved: {unresolved}",
            entry.role
        ),
    ))
}

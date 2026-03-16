use pi::sdk::{ContentBlock, Message, UserContent};
use pi_lynx_sdk::{
    HistoryWarningKind, HostContentBlock, HostTranscriptEntry, HostTranscriptRole,
    reconstruct_history,
};
use pretty_assertions::assert_eq;
use serde_json::json;

/// WHY: continue/retry flows only work if assistant tool calls and tool results
/// are reconstructed in the exact order Pi expects from its in-memory history.
#[test]
fn reconstruct_history_preserves_tool_ordering() {
    let transcript = vec![
        HostTranscriptEntry {
            role: HostTranscriptRole::User,
            message_id: Some("u1".to_string()),
            tool_call_id: None,
            tool_name: None,
            custom_type: None,
            content: vec![HostContentBlock::Text {
                text: "inspect README".to_string(),
            }],
            is_error: false,
            timestamp_ms: Some(1),
        },
        HostTranscriptEntry {
            role: HostTranscriptRole::Assistant,
            message_id: Some("a1".to_string()),
            tool_call_id: None,
            tool_name: None,
            custom_type: None,
            content: vec![
                HostContentBlock::Text {
                    text: "I'll read that.".to_string(),
                },
                HostContentBlock::ToolCall {
                    tool_call_id: "call_1".to_string(),
                    tool_name: "read".to_string(),
                    arguments: json!({ "path": "README.md" }),
                },
            ],
            is_error: false,
            timestamp_ms: Some(2),
        },
        HostTranscriptEntry {
            role: HostTranscriptRole::ToolResult,
            message_id: Some("t1".to_string()),
            tool_call_id: Some("call_1".to_string()),
            tool_name: Some("read".to_string()),
            custom_type: None,
            content: vec![HostContentBlock::Text {
                text: "README contents".to_string(),
            }],
            is_error: false,
            timestamp_ms: Some(3),
        },
        HostTranscriptEntry {
            role: HostTranscriptRole::Assistant,
            message_id: Some("a2".to_string()),
            tool_call_id: None,
            tool_name: None,
            custom_type: None,
            content: vec![HostContentBlock::Text {
                text: "Done.".to_string(),
            }],
            is_error: false,
            timestamp_ms: Some(4),
        },
    ];

    let result = reconstruct_history(&transcript).expect("history reconstructs");

    assert_eq!(result.warnings.len(), 0);
    assert_eq!(result.messages.len(), 4);
    assert!(matches!(
        &result.messages[0],
        Message::User(message) if matches!(&message.content, UserContent::Text(text) if text == "inspect README")
    ));
    assert!(matches!(
        &result.messages[1],
        Message::Assistant(message)
            if matches!(message.content[1], ContentBlock::ToolCall(ref call)
                if call.id == "call_1" && call.name == "read")
    ));
    assert!(matches!(
        &result.messages[2],
        Message::ToolResult(message) if message.tool_call_id == "call_1" && message.tool_name == "read"
    ));
}

/// WHY: tool results without a matching assistant tool call are structurally
/// ambiguous and would corrupt continuation state if accepted.
#[test]
fn reconstruct_history_rejects_unknown_tool_result() {
    let transcript = vec![tool_result_entry(
        "t1",
        "missing",
        "read",
        "README contents",
        1,
    )];

    let error = reconstruct_history(&transcript).expect_err("unknown tool result must fail");
    assert_eq!(error.kind(), pi_lynx_sdk::EmbedErrorKind::InvalidTranscript);
    assert!(error.to_string().contains("unknown tool_call_id 'missing'"));
}

/// WHY: partial-turn continuation input cannot skip directly from an assistant
/// tool call back to normal transcript roles without corrupting provider state.
#[test]
fn reconstruct_history_rejects_non_tool_entries_with_unresolved_tool_calls() {
    let transcript = vec![
        assistant_tool_call_entry("a1", "call_1", "read", 1),
        HostTranscriptEntry {
            role: HostTranscriptRole::User,
            message_id: Some("u2".to_string()),
            tool_call_id: None,
            tool_name: None,
            custom_type: None,
            content: vec![HostContentBlock::Text {
                text: "try again".to_string(),
            }],
            is_error: false,
            timestamp_ms: Some(2),
        },
    ];

    let error = reconstruct_history(&transcript).expect_err("unresolved tool calls must fail");

    assert_eq!(error.kind(), pi_lynx_sdk::EmbedErrorKind::InvalidTranscript);
    assert!(error.to_string().contains("unresolved"));
    assert!(error.to_string().contains("call_1"));
}

/// WHY: hosts can persist interrupted turns, but the embed layer must reject
/// transcripts that end before the assistant's requested tool work is replayed.
#[test]
fn reconstruct_history_rejects_transcript_ending_with_unresolved_tool_calls() {
    let transcript = vec![assistant_tool_call_entry("a1", "call_1", "read", 1)];

    let error =
        reconstruct_history(&transcript).expect_err("unfinished tool-use transcript must fail");

    assert_eq!(error.kind(), pi_lynx_sdk::EmbedErrorKind::InvalidTranscript);
    assert!(
        error
            .to_string()
            .contains("transcript ended with unresolved assistant")
    );
    assert!(error.to_string().contains("call_1"));
}

/// WHY: Pi custom messages are text-only, so the embed layer needs to preserve
/// the text payload while warning when richer host-only blocks are dropped.
#[test]
fn reconstruct_history_warns_when_custom_blocks_are_dropped() {
    let transcript = vec![HostTranscriptEntry {
        role: HostTranscriptRole::Custom,
        message_id: Some("c1".to_string()),
        tool_call_id: None,
        tool_name: None,
        custom_type: Some("lynx_note".to_string()),
        content: vec![
            HostContentBlock::Text {
                text: "note".to_string(),
            },
            HostContentBlock::Image {
                mime_type: "image/png".to_string(),
                data: vec![1, 2, 3],
            },
        ],
        is_error: false,
        timestamp_ms: Some(1),
    }];

    let result = reconstruct_history(&transcript).expect("custom transcript reconstructs");

    assert_eq!(result.warnings.len(), 1);
    assert_eq!(
        result.warnings[0].kind,
        HistoryWarningKind::CustomContentBlockDropped
    );
    assert!(matches!(
        &result.messages[0],
        Message::Custom(message) if message.content == "note" && message.custom_type == "lynx_note"
    ));
}

fn assistant_tool_call_entry(
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
        content: vec![HostContentBlock::ToolCall {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: json!({ "path": "README.md" }),
        }],
        is_error: false,
        timestamp_ms: Some(timestamp_ms),
    }
}

fn tool_result_entry(
    message_id: &str,
    tool_call_id: &str,
    tool_name: &str,
    text: &str,
    timestamp_ms: i64,
) -> HostTranscriptEntry {
    HostTranscriptEntry {
        role: HostTranscriptRole::ToolResult,
        message_id: Some(message_id.to_string()),
        tool_call_id: Some(tool_call_id.to_string()),
        tool_name: Some(tool_name.to_string()),
        custom_type: None,
        content: vec![HostContentBlock::Text {
            text: text.to_string(),
        }],
        is_error: false,
        timestamp_ms: Some(timestamp_ms),
    }
}

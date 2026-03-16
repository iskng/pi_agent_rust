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
    let transcript = vec![HostTranscriptEntry {
        role: HostTranscriptRole::ToolResult,
        message_id: Some("t1".to_string()),
        tool_call_id: Some("missing".to_string()),
        tool_name: Some("read".to_string()),
        custom_type: None,
        content: vec![HostContentBlock::Text {
            text: "README contents".to_string(),
        }],
        is_error: false,
        timestamp_ms: Some(1),
    }];

    let error = reconstruct_history(&transcript).expect_err("unknown tool result must fail");
    assert_eq!(error.kind(), pi_lynx_sdk::EmbedErrorKind::InvalidTranscript);
    assert!(error.to_string().contains("unknown tool_call_id 'missing'"));
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

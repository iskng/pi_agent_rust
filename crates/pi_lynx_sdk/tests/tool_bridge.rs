use async_trait::async_trait;
use pi::sdk::{ContentBlock, TextContent};
use pi_lynx_sdk::{
    HostToolAdapter, HostToolDefinition, HostToolError, HostToolKind, HostToolOutput,
    HostToolRequest, HostToolUpdate, RuntimeMetadata, ToolPolicy, build_tool_registry,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
struct RecordingTool {
    kind: HostToolKind,
    definition: HostToolDefinition,
    requests: Arc<Mutex<Vec<HostToolRequest>>>,
    updates: Vec<HostToolUpdate>,
    result: std::result::Result<HostToolOutput, HostToolError>,
}

#[async_trait]
impl HostToolAdapter for RecordingTool {
    fn kind(&self) -> HostToolKind {
        self.kind
    }

    fn definition(&self) -> HostToolDefinition {
        self.definition.clone()
    }

    async fn execute(
        &self,
        request: HostToolRequest,
        on_update: Option<Box<dyn Fn(HostToolUpdate) + Send + Sync>>,
    ) -> std::result::Result<HostToolOutput, HostToolError> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);

        if let Some(on_update) = on_update {
            for update in &self.updates {
                on_update(update.clone());
            }
        }

        self.result.clone()
    }
}

/// WHY: the embed bridge must forward raw JSON arguments, tool-call identity,
/// runtime metadata, and streaming updates exactly so Lynx remains the source
/// of truth for host-side tool execution and progress reporting.
#[test]
fn build_tool_registry_translates_requests_and_streaming_updates() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let adapter: Arc<dyn HostToolAdapter> = Arc::new(RecordingTool {
        kind: HostToolKind::Read,
        definition: HostToolDefinition {
            name: "read".to_string(),
            label: "Read".to_string(),
            description: "Read through the host.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                }
            }),
        },
        requests: Arc::clone(&requests),
        updates: vec![HostToolUpdate {
            content: vec![ContentBlock::Text(TextContent::new("partial"))],
            details: Some(json!({ "stage": "streaming" })),
        }],
        result: Ok(HostToolOutput {
            content: vec![ContentBlock::Text(TextContent::new("done"))],
            details: Some(json!({ "lines": 1 })),
            is_error: false,
        }),
    });

    let registry = build_tool_registry(
        &ToolPolicy::default(),
        &RuntimeMetadata {
            conversation_id: Some("conv-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            ..RuntimeMetadata::default()
        },
        &[adapter],
    )
    .expect("registry");
    let tool = registry.get("read").expect("read tool");
    let streamed_updates = Arc::new(Mutex::new(Vec::new()));
    let streamed_updates_capture = Arc::clone(&streamed_updates);

    let output = runtime.block_on(async move {
        tool.execute(
            "call-1",
            json!({ "path": "README.md" }),
            Some(Box::new(move |update| {
                streamed_updates_capture
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(update);
            })),
        )
        .await
        .expect("tool output")
    });

    let requests = requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tool_call_id, "call-1");
    assert_eq!(requests[0].tool_name, "read");
    assert_eq!(requests[0].kind, HostToolKind::Read);
    assert_eq!(requests[0].input, json!({ "path": "README.md" }));
    assert_eq!(
        requests[0].runtime_metadata.conversation_id.as_deref(),
        Some("conv-1")
    );

    let streamed_updates = streamed_updates
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(streamed_updates.len(), 1);
    assert_eq!(
        streamed_updates[0].details,
        Some(json!({ "stage": "streaming" }))
    );
    assert!(matches!(
        streamed_updates[0].content.as_slice(),
        [ContentBlock::Text(text)] if text.text == "partial"
    ));
    assert_eq!(output.details, Some(json!({ "lines": 1 })));
    assert!(!output.is_error);
}

/// WHY: host policy can reject a tool call after the model asks for it, and
/// that denial must become a structured error result instead of crashing the
/// turn or silently pretending the tool succeeded.
#[test]
fn denied_host_tool_becomes_error_output() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");
    let adapter: Arc<dyn HostToolAdapter> = Arc::new(RecordingTool {
        kind: HostToolKind::Exec,
        definition: HostToolDefinition {
            name: "exec".to_string(),
            label: "Exec".to_string(),
            description: "Execute through host approval.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            }),
        },
        requests: Arc::new(Mutex::new(Vec::new())),
        updates: Vec::new(),
        result: Err(HostToolError::denied_with_details(
            "approval required",
            json!({ "approvalState": "missing" }),
        )),
    });

    let registry = build_tool_registry(
        &ToolPolicy {
            allowed_tools: vec![HostToolKind::Exec],
            allow_mutations: false,
            allow_exec: true,
        },
        &RuntimeMetadata::default(),
        &[adapter],
    )
    .expect("registry");
    let tool = registry.get("exec").expect("exec tool");

    let output = runtime.block_on(async move {
        tool.execute("call-2", json!({ "command": "git status" }), None)
            .await
            .expect("tool output")
    });

    assert!(output.is_error);
    assert_eq!(
        output
            .details
            .as_ref()
            .and_then(|details| details.get("hostToolErrorKind"))
            .and_then(serde_json::Value::as_str),
        Some("denied")
    );
    assert!(matches!(
        output.content.as_slice(),
        [ContentBlock::Text(text)] if text.text.contains("Denied: approval required")
    ));
}

/// WHY: transport or execution failures inside the host tool layer still need
/// to become deterministic tool results so the model sees a normal error path
/// instead of losing the turn to an adapter crash.
#[test]
fn failed_host_tool_becomes_error_output() {
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("runtime");
    let adapter: Arc<dyn HostToolAdapter> = Arc::new(RecordingTool {
        kind: HostToolKind::Search,
        definition: HostToolDefinition {
            name: "search".to_string(),
            label: "Search".to_string(),
            description: "Search through host indexing.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" }
                }
            }),
        },
        requests: Arc::new(Mutex::new(Vec::new())),
        updates: Vec::new(),
        result: Err(HostToolError::failed_with_details(
            "index backend unavailable",
            json!({ "retryable": true }),
        )),
    });

    let registry = build_tool_registry(
        &ToolPolicy::default(),
        &RuntimeMetadata::default(),
        &[adapter],
    )
    .expect("registry");
    let tool = registry.get("search").expect("search tool");

    let output = runtime.block_on(async move {
        tool.execute("call-3", json!({ "pattern": "TODO" }), None)
            .await
            .expect("tool output")
    });

    assert!(output.is_error);
    assert_eq!(
        output
            .details
            .as_ref()
            .and_then(|details| details.get("hostToolErrorKind"))
            .and_then(serde_json::Value::as_str),
        Some("failed")
    );
    assert!(matches!(
        output.content.as_slice(),
        [ContentBlock::Text(text)] if text.text.contains("Error: index backend unavailable")
    ));
}

/// WHY: policy filtering must keep disallowed mutating tools out of the model
/// surface entirely so embed mode never exposes Pi-style local mutation paths
/// through a configuration mistake.
#[test]
fn build_tool_registry_filters_tools_disallowed_by_policy() {
    let read: Arc<dyn HostToolAdapter> = Arc::new(RecordingTool {
        kind: HostToolKind::Read,
        definition: HostToolDefinition {
            name: "read".to_string(),
            label: "Read".to_string(),
            description: "Read through host.".to_string(),
            parameters: json!({ "type": "object" }),
        },
        requests: Arc::new(Mutex::new(Vec::new())),
        updates: Vec::new(),
        result: Ok(HostToolOutput {
            content: Vec::new(),
            details: None,
            is_error: false,
        }),
    });
    let write: Arc<dyn HostToolAdapter> = Arc::new(RecordingTool {
        kind: HostToolKind::Write,
        definition: HostToolDefinition {
            name: "write".to_string(),
            label: "Write".to_string(),
            description: "Write through host.".to_string(),
            parameters: json!({ "type": "object" }),
        },
        requests: Arc::new(Mutex::new(Vec::new())),
        updates: Vec::new(),
        result: Ok(HostToolOutput {
            content: Vec::new(),
            details: None,
            is_error: false,
        }),
    });

    let registry = build_tool_registry(
        &ToolPolicy::default(),
        &RuntimeMetadata::default(),
        &[read, write],
    )
    .expect("registry");

    assert!(registry.get("read").is_some());
    assert!(registry.get("write").is_none());
    assert_eq!(registry.tools().len(), 1);
}

/// WHY: disabled tools should be invisible to registry assembly so hosts can
/// keep a broader adapter pool without tripping validation on tools this turn
/// does not actually expose.
#[test]
fn build_tool_registry_skips_disabled_tools_before_validation() {
    let read: Arc<dyn HostToolAdapter> = Arc::new(RecordingTool {
        kind: HostToolKind::Read,
        definition: HostToolDefinition {
            name: "read".to_string(),
            label: "Read".to_string(),
            description: "Read through host.".to_string(),
            parameters: json!({ "type": "object" }),
        },
        requests: Arc::new(Mutex::new(Vec::new())),
        updates: Vec::new(),
        result: Ok(HostToolOutput {
            content: Vec::new(),
            details: None,
            is_error: false,
        }),
    });
    let disabled_write: Arc<dyn HostToolAdapter> = Arc::new(RecordingTool {
        kind: HostToolKind::Write,
        definition: HostToolDefinition {
            name: "".to_string(),
            label: "".to_string(),
            description: "".to_string(),
            parameters: json!("invalid"),
        },
        requests: Arc::new(Mutex::new(Vec::new())),
        updates: Vec::new(),
        result: Ok(HostToolOutput {
            content: Vec::new(),
            details: None,
            is_error: false,
        }),
    });

    let registry = build_tool_registry(
        &ToolPolicy::default(),
        &RuntimeMetadata::default(),
        &[disabled_write, read],
    )
    .expect("disabled invalid tool should be skipped");

    assert!(registry.get("read").is_some());
    assert_eq!(registry.tools().len(), 1);
}

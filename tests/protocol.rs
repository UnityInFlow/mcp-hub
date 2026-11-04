//! Protocol serialization and deserialization tests.
//!
//! Covers all MCP introspection request/response types added in Phase 3 Plan 01.

use mcp_hub::mcp::protocol::{
    initialize_request, prompts_list_request, resources_list_request, tools_list_request,
    InitializeResult, JsonRpcNotification, McpPrompt, McpResource, McpTool, PromptsListResult,
    ResourcesListResult, ToolsListResult,
};

// ─────────────────────────────────────────────────────────────────────────────
// initialize_request serialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn initialize_request_serialization() {
    let req = initialize_request(1);
    let json = serde_json::to_string(&req).expect("should serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("should parse");

    assert_eq!(value["jsonrpc"], "2.0", "jsonrpc field should be 2.0");
    assert_eq!(value["method"], "initialize", "method should be initialize");
    assert_eq!(value["id"], 1, "id should match");

    let params = &value["params"];
    assert_eq!(
        params["protocolVersion"], "2024-11-05",
        "protocolVersion should be 2024-11-05"
    );
    assert_eq!(
        params["clientInfo"]["name"], "mcp-hub",
        "clientInfo.name should be mcp-hub"
    );
    assert!(
        params["clientInfo"]["version"].is_string(),
        "clientInfo.version should be a string"
    );
}

#[test]
fn initialize_request_no_id_collision() {
    let req1 = initialize_request(1);
    let req2 = initialize_request(2);

    let json1 = serde_json::to_string(&req1).expect("should serialize");
    let json2 = serde_json::to_string(&req2).expect("should serialize");

    let v1: serde_json::Value = serde_json::from_str(&json1).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&json2).unwrap();

    assert_ne!(
        v1["id"], v2["id"],
        "different requests should have different ids"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// tools_list_request serialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tools_list_request_serialization() {
    let req = tools_list_request(10);
    let json = serde_json::to_string(&req).expect("should serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("should parse");

    assert_eq!(value["method"], "tools/list", "method should be tools/list");
    assert_eq!(value["id"], 10, "id should match");
    assert!(value["jsonrpc"].is_string());
    // params should be absent (null or missing) for no-params requests.
    assert!(
        value.get("params").is_none() || value["params"].is_null(),
        "params should be absent or null"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// resources_list_request serialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn resources_list_request_serialization() {
    let req = resources_list_request(20);
    let json = serde_json::to_string(&req).expect("should serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("should parse");

    assert_eq!(
        value["method"], "resources/list",
        "method should be resources/list"
    );
    assert_eq!(value["id"], 20, "id should match");
}

// ─────────────────────────────────────────────────────────────────────────────
// prompts_list_request serialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn prompts_list_request_serialization() {
    let req = prompts_list_request(30);
    let json = serde_json::to_string(&req).expect("should serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("should parse");

    assert_eq!(
        value["method"], "prompts/list",
        "method should be prompts/list"
    );
    assert_eq!(value["id"], 30, "id should match");
}

// ─────────────────────────────────────────────────────────────────────────────
// JsonRpcNotification::initialized serialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn notifications_initialized_serialization() {
    let notif = JsonRpcNotification::initialized();
    let json = serde_json::to_string(&notif).expect("should serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("should parse");

    assert_eq!(
        value["method"], "notifications/initialized",
        "method should be notifications/initialized"
    );
    assert_eq!(value["jsonrpc"], "2.0");
    // Notifications must NOT have an "id" field.
    assert!(
        value.get("id").is_none(),
        "notification should not have an id field"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// InitializeResult deserialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn initialize_result_deserialization() {
    let json = r#"{
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {},
            "resources": {},
            "prompts": {}
        },
        "serverInfo": {
            "name": "my-mcp-server",
            "version": "1.2.3"
        }
    }"#;

    let result: InitializeResult = serde_json::from_str(json).expect("should deserialize");

    assert_eq!(result.protocol_version, "2024-11-05");
    assert!(result.capabilities.tools.is_some());
    assert!(result.capabilities.resources.is_some());
    assert!(result.capabilities.prompts.is_some());
    let server_info = result.server_info.expect("server_info should be Some");
    assert_eq!(server_info.name, "my-mcp-server");
    assert_eq!(server_info.version.as_deref(), Some("1.2.3"));
}

#[test]
fn initialize_result_minimal_deserialization() {
    // Minimal response: no serverInfo, no capabilities fields.
    let json = r#"{
        "protocolVersion": "2024-11-05",
        "capabilities": {}
    }"#;

    let result: InitializeResult = serde_json::from_str(json).expect("should deserialize");

    assert_eq!(result.protocol_version, "2024-11-05");
    assert!(result.capabilities.tools.is_none());
    assert!(result.capabilities.resources.is_none());
    assert!(result.capabilities.prompts.is_none());
    assert!(result.server_info.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// ToolsListResult deserialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tools_list_result_deserialization() {
    let json = r#"{
        "tools": [
            {
                "name": "read_file",
                "description": "Read a file from disk",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            },
            {
                "name": "write_file",
                "description": null,
                "inputSchema": null
            }
        ]
    }"#;

    let result: ToolsListResult = serde_json::from_str(json).expect("should deserialize");

    assert_eq!(result.tools.len(), 2, "should have 2 tools");
    assert_eq!(result.tools[0].name, "read_file");
    assert_eq!(
        result.tools[0].description.as_deref(),
        Some("Read a file from disk")
    );
    assert!(result.tools[0].input_schema.is_some());
    assert_eq!(result.tools[1].name, "write_file");
    assert!(result.tools[1].description.is_none());
    assert!(result.tools[1].input_schema.is_none());
}

#[test]
fn tools_list_result_empty_tools() {
    let json = r#"{"tools": []}"#;
    let result: ToolsListResult = serde_json::from_str(json).expect("should deserialize");
    assert!(result.tools.is_empty());
}

#[test]
fn mcp_tool_roundtrip() {
    let tool = McpTool {
        name: "my_tool".to_string(),
        description: Some("does stuff".to_string()),
        input_schema: Some(serde_json::json!({"type": "object"})),
    };
    let json = serde_json::to_string(&tool).expect("should serialize");
    let deserialized: McpTool = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(deserialized.name, tool.name);
    assert_eq!(deserialized.description, tool.description);
}

// ─────────────────────────────────────────────────────────────────────────────
// ResourcesListResult deserialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn resources_list_result_deserialization() {
    let json = r#"{
        "resources": [
            {
                "uri": "file:///home/user/doc.txt",
                "name": "doc.txt",
                "description": "A text document",
                "mimeType": "text/plain"
            }
        ]
    }"#;

    let result: ResourcesListResult = serde_json::from_str(json).expect("should deserialize");

    assert_eq!(result.resources.len(), 1);
    assert_eq!(result.resources[0].uri, "file:///home/user/doc.txt");
    assert_eq!(result.resources[0].name, "doc.txt");
    assert_eq!(
        result.resources[0].description.as_deref(),
        Some("A text document")
    );
    assert_eq!(result.resources[0].mime_type.as_deref(), Some("text/plain"));
}

#[test]
fn mcp_resource_roundtrip() {
    let resource = McpResource {
        uri: "file:///tmp/test.json".to_string(),
        name: "test.json".to_string(),
        description: None,
        mime_type: Some("application/json".to_string()),
    };
    let json = serde_json::to_string(&resource).expect("should serialize");
    let deserialized: McpResource = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(deserialized.uri, resource.uri);
    assert_eq!(deserialized.mime_type, resource.mime_type);
}

// ─────────────────────────────────────────────────────────────────────────────
// PromptsListResult deserialization
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn prompts_list_result_deserialization() {
    let json = r#"{
        "prompts": [
            {
                "name": "code_review",
                "description": "Review code for issues",
                "arguments": [
                    {
                        "name": "code",
                        "description": "The code to review",
                        "required": true
                    },
                    {
                        "name": "language",
                        "description": "Programming language",
                        "required": false
                    }
                ]
            },
            {
                "name": "summarize",
                "description": null,
                "arguments": null
            }
        ]
    }"#;

    let result: PromptsListResult = serde_json::from_str(json).expect("should deserialize");

    assert_eq!(result.prompts.len(), 2);
    assert_eq!(result.prompts[0].name, "code_review");
    let args = result.prompts[0]
        .arguments
        .as_ref()
        .expect("arguments should be Some");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].name, "code");
    assert_eq!(args[0].required, Some(true));
    assert_eq!(args[1].name, "language");
    assert_eq!(args[1].required, Some(false));

    assert_eq!(result.prompts[1].name, "summarize");
    assert!(result.prompts[1].arguments.is_none());
}

#[test]
fn mcp_prompt_roundtrip() {
    let prompt = McpPrompt {
        name: "my_prompt".to_string(),
        description: Some("a prompt".to_string()),
        arguments: None,
    };
    let json = serde_json::to_string(&prompt).expect("should serialize");
    let deserialized: McpPrompt = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(deserialized.name, prompt.name);
    assert_eq!(deserialized.description, prompt.description);
    assert!(deserialized.arguments.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// JsonRpcResponse strict parsing (issue #24)
// ─────────────────────────────────────────────────────────────────────────────

mod response_parsing {
    use mcp_hub::mcp::protocol::JsonRpcResponse;

    fn parse(json: &str) -> Result<JsonRpcResponse, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn accepts_result_response() {
        let resp = parse(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).expect("valid response");
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn accepts_null_result() {
        let resp = parse(r#"{"jsonrpc":"2.0","id":1,"result":null}"#).expect("valid response");
        assert_eq!(resp.result, Some(serde_json::Value::Null));
    }

    #[test]
    fn accepts_error_response() {
        let resp = parse(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"x"}}"#)
            .expect("valid response");
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
    }

    #[test]
    fn rejects_neither_result_nor_error() {
        assert!(parse(r#"{"jsonrpc":"2.0","id":1}"#).is_err());
    }

    #[test]
    fn rejects_both_result_and_error() {
        assert!(parse(r#"{"jsonrpc":"2.0","id":1,"result":{},"error":{}}"#).is_err());
    }

    #[test]
    fn rejects_server_initiated_request() {
        assert!(parse(r#"{"jsonrpc":"2.0","id":3,"method":"roots/list","params":{}}"#).is_err());
    }

    #[test]
    fn rejects_notification() {
        assert!(parse(r#"{"jsonrpc":"2.0","method":"notifications/progress"}"#).is_err());
    }

    #[test]
    fn rejects_method_even_alongside_result() {
        assert!(parse(r#"{"jsonrpc":"2.0","id":1,"method":"x","result":{}}"#).is_err());
    }

    #[test]
    fn rejects_missing_and_null_id() {
        assert!(parse(r#"{"jsonrpc":"2.0","result":{}}"#).is_err());
        assert!(parse(r#"{"jsonrpc":"2.0","id":null,"result":{}}"#).is_err());
    }

    #[test]
    fn accepts_string_id() {
        assert!(parse(r#"{"jsonrpc":"2.0","id":"abc","result":{}}"#).is_ok());
    }
}

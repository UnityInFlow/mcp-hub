// Phase 3 introspection types — not all are wired yet; allow dead_code
// until Plans 02 and 03 use these types.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Generic JSON-RPC 2.0 request with typed params.
///
/// Use the constructor helpers (`initialize_request`, `tools_list_request`, etc.)
/// instead of constructing this directly.
#[derive(Debug, Serialize)]
pub struct JsonRpcRequest<T: Serialize> {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<T>,
}

/// JSON-RPC 2.0 notification (no `id` — fire-and-forget, no response expected).
#[derive(Debug, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: &'static str,
}

impl JsonRpcNotification {
    /// Build the `notifications/initialized` notification sent after a successful
    /// `initialize` handshake to signal the client is ready.
    pub fn initialized() -> Self {
        Self {
            jsonrpc: "2.0",
            method: "notifications/initialized",
        }
    }
}

/// JSON-RPC 2.0 ping request sent to MCP servers for health checking.
///
/// Phase 2 only uses `ping`. Phase 3 introspection uses `initialize`, `tools/list`, etc.
#[derive(Debug, Serialize)]
pub struct PingRequest {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub id: u64,
}

impl PingRequest {
    pub fn new(id: u64) -> Self {
        Self {
            jsonrpc: "2.0",
            method: "ping",
            id,
        }
    }
}

/// JSON-RPC 2.0 request/response identifier.
///
/// The hub only ever sends numeric ids, but a peer may legally use strings, so
/// both are modelled explicitly instead of silently discarding the latter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(u64),
    String(String),
}

/// Why a JSON object was rejected as a JSON-RPC response.
#[derive(Debug, thiserror::Error)]
pub enum InvalidResponse {
    #[error("message carries a `method` (request or notification, not a response)")]
    HasMethod,
    #[error("response has no `id`")]
    MissingId,
    #[error("response `id` is neither an unsigned integer nor a string")]
    InvalidId,
    #[error("response has neither `result` nor `error`")]
    MissingPayload,
    #[error("response has both `result` and `error`")]
    AmbiguousPayload,
}

/// Generic JSON-RPC 2.0 response.
///
/// Deserialisation is strict: a message is only a response if it has no
/// `method`, has an `id`, and carries exactly one of `result` / `error`
/// (a `null` result counts as present). Server-initiated requests such as
/// `roots/list` therefore never parse as a response to one of the hub's own
/// requests that happens to share their id.
///
/// Phase 2 validates `id` correlation and checks for `error`.
/// Phase 3 parses `result` for introspection data via `serde_json::from_value`.
#[derive(Debug, Deserialize)]
#[serde(try_from = "serde_json::Map<String, serde_json::Value>")]
pub struct JsonRpcResponse {
    pub id: JsonRpcId,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}

impl TryFrom<serde_json::Map<String, serde_json::Value>> for JsonRpcResponse {
    type Error = InvalidResponse;

    fn try_from(mut map: serde_json::Map<String, serde_json::Value>) -> Result<Self, Self::Error> {
        if map.contains_key("method") {
            return Err(InvalidResponse::HasMethod);
        }
        let id_value = map.remove("id").ok_or(InvalidResponse::MissingId)?;
        let id = serde_json::from_value::<JsonRpcId>(id_value)
            .map_err(|_| InvalidResponse::InvalidId)?;
        let result = map.remove("result");
        let error = map.remove("error");
        match (&result, &error) {
            (None, None) => Err(InvalidResponse::MissingPayload),
            (Some(_), Some(_)) => Err(InvalidResponse::AmbiguousPayload),
            (Some(_), None) | (None, Some(_)) => Ok(Self { id, result, error }),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Initialize request params
// ─────────────────────────────────────────────────────────────────────────────

/// Params for the MCP `initialize` request.
#[derive(Debug, Serialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: serde_json::Value,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}

/// Identifies this MCP client to the server during `initialize`.
#[derive(Debug, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Initialize response result types
// ─────────────────────────────────────────────────────────────────────────────

/// Deserialized result from a successful `initialize` response.
#[derive(Debug, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: Option<ServerInfo>,
}

/// The capability flags advertised by the MCP server in `initialize`.
#[derive(Debug, Deserialize)]
pub struct ServerCapabilities {
    pub tools: Option<serde_json::Value>,
    pub resources: Option<serde_json::Value>,
    pub prompts: Option<serde_json::Value>,
}

/// Human-readable identity of the MCP server.
#[derive(Debug, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// tools/list result types
// ─────────────────────────────────────────────────────────────────────────────

/// Deserialized result from a `tools/list` response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolsListResult {
    pub tools: Vec<McpTool>,
}

/// A single tool exposed by an MCP server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// resources/list result types
// ─────────────────────────────────────────────────────────────────────────────

/// Deserialized result from a `resources/list` response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourcesListResult {
    pub resources: Vec<McpResource>,
}

/// A single resource exposed by an MCP server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// prompts/list result types
// ─────────────────────────────────────────────────────────────────────────────

/// Deserialized result from a `prompts/list` response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PromptsListResult {
    pub prompts: Vec<McpPrompt>,
}

/// A single prompt template exposed by an MCP server.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpPrompt {
    pub name: String,
    pub description: Option<String>,
    pub arguments: Option<Vec<PromptArgument>>,
}

/// An argument accepted by a prompt template.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Request constructor helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build an MCP `initialize` request with protocol version `2024-11-05`.
pub fn initialize_request(id: u64) -> JsonRpcRequest<InitializeParams> {
    JsonRpcRequest {
        jsonrpc: "2.0",
        method: "initialize",
        id,
        params: Some(InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: serde_json::json!({}),
            client_info: ClientInfo {
                name: "mcp-hub".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        }),
    }
}

/// Build a `tools/list` request.
pub fn tools_list_request(id: u64) -> JsonRpcRequest<()> {
    JsonRpcRequest {
        jsonrpc: "2.0",
        method: "tools/list",
        id,
        params: None,
    }
}

/// Build a `resources/list` request.
pub fn resources_list_request(id: u64) -> JsonRpcRequest<()> {
    JsonRpcRequest {
        jsonrpc: "2.0",
        method: "resources/list",
        id,
        params: None,
    }
}

/// Build a `prompts/list` request.
pub fn prompts_list_request(id: u64) -> JsonRpcRequest<()> {
    JsonRpcRequest {
        jsonrpc: "2.0",
        method: "prompts/list",
        id,
        params: None,
    }
}

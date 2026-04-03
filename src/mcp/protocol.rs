#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// JSON-RPC 2.0 ping request sent to MCP servers for health checking.
///
/// Phase 2 only uses `ping`. Phase 3 will add `initialize`, `tools/list`,
/// `resources/list`, `prompts/list` request types.
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

/// Generic JSON-RPC 2.0 response.
///
/// Phase 2 only validates `id` correlation and checks for `error`.
/// Phase 3 will parse `result` for introspection data.
#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub id: u64,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}

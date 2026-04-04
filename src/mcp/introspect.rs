//! MCP capability introspection — runs the full `initialize` handshake followed by
//! concurrent `tools/list`, `resources/list`, and `prompts/list` requests.
//!
//! Entry point: [`run_introspection`]. Called once per process spawn when the server
//! first reaches [`crate::types::HealthStatus::Healthy`].

use std::sync::Arc;

use crate::mcp::dispatcher::{
    send_notification, send_request, IdAllocator, PendingMap, SharedStdin,
};
use crate::mcp::protocol::{
    initialize_request, prompts_list_request, resources_list_request, tools_list_request,
    InitializeResult, JsonRpcNotification, JsonRpcResponse, McpPrompt, McpResource, McpTool,
    PromptsListResult, ResourcesListResult, ServerCapabilities, ToolsListResult,
};
use crate::types::{McpCapabilities, ServerSnapshot};

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Run the full MCP introspection sequence for a server.
///
/// Steps:
/// 1. Send `initialize` request and await the response.
/// 2. Send `notifications/initialized` fire-and-forget.
/// 3. Concurrently send `tools/list`, `resources/list`, and `prompts/list` for any
///    capability families declared by the server in its `initialize` response.
/// 4. Store the collected capabilities in the `ServerSnapshot` via `snapshot_tx`.
///
/// # Errors
/// - `initialize` request fails or returns an error response.
/// - Failed list requests are handled gracefully (logged as warnings, not propagated).
pub async fn run_introspection(
    server_name: &str,
    stdin: &SharedStdin,
    pending: &PendingMap,
    id_alloc: &Arc<IdAllocator>,
    snapshot_tx: &tokio::sync::watch::Sender<ServerSnapshot>,
) -> anyhow::Result<McpCapabilities> {
    // Step 1: Send initialize request and await response.
    let init_id = id_alloc.next_id();
    let init_req = initialize_request(init_id);
    let init_response = send_request(stdin, pending, init_id, &init_req, 10).await?;

    // Parse the initialize result to check server capabilities.
    let init_result: InitializeResult = match init_response.result {
        Some(value) => serde_json::from_value(value)
            .map_err(|e| anyhow::anyhow!("Failed to parse initialize result: {e}"))?,
        None => {
            if let Some(err) = init_response.error {
                anyhow::bail!("Server returned error on initialize: {err}");
            }
            anyhow::bail!("Server returned empty initialize response");
        }
    };

    tracing::info!(
        server = %server_name,
        protocol = %init_result.protocol_version,
        server_name = ?init_result.server_info.as_ref().map(|s| &s.name),
        "MCP initialize handshake complete"
    );

    // Step 2: Send notifications/initialized (fire-and-forget).
    let notification = JsonRpcNotification::initialized();
    send_notification(stdin, &notification).await?;

    // Step 3: Send concurrent list requests based on declared capabilities.
    let caps = &init_result.capabilities;
    let capabilities = fetch_capabilities(server_name, stdin, pending, id_alloc, caps).await;

    // Step 4: Update the snapshot with introspected capabilities.
    let caps_clone = capabilities.clone();
    snapshot_tx.send_modify(|s| {
        s.capabilities = caps_clone;
    });

    tracing::info!(
        server = %server_name,
        tools = capabilities.tools.len(),
        resources = capabilities.resources.len(),
        prompts = capabilities.prompts.len(),
        "Introspection complete"
    );

    Ok(capabilities)
}

// ─────────────────────────────────────────────────────────────────────────────
// Concurrent list requests
// ─────────────────────────────────────────────────────────────────────────────

/// Send `tools/list`, `resources/list`, and `prompts/list` concurrently, conditioned
/// on the capability flags declared by the server in its `initialize` response.
///
/// Any request that fails (timeout, IO error, error response, or parse error) is
/// handled gracefully: a warning is logged and an empty `Vec` is returned for that
/// capability family. The other families are unaffected.
async fn fetch_capabilities(
    server_name: &str,
    stdin: &SharedStdin,
    pending: &PendingMap,
    id_alloc: &Arc<IdAllocator>,
    server_caps: &ServerCapabilities,
) -> McpCapabilities {
    // Allocate IDs upfront so they are distinct.
    let tools_id = id_alloc.next_id();
    let resources_id = id_alloc.next_id();
    let prompts_id = id_alloc.next_id();

    // Capture capability flags so they can be moved into async blocks.
    let has_tools = server_caps.tools.is_some();
    let has_resources = server_caps.resources.is_some();
    let has_prompts = server_caps.prompts.is_some();

    // Await all three concurrently via tokio::join! — each async block is self-contained
    // so the request value lives for the full duration of the await.
    let (tools_result, resources_result, prompts_result) = tokio::join!(
        async {
            if has_tools {
                let req = tools_list_request(tools_id);
                Some(send_request(stdin, pending, tools_id, &req, 10).await)
            } else {
                None
            }
        },
        async {
            if has_resources {
                let req = resources_list_request(resources_id);
                Some(send_request(stdin, pending, resources_id, &req, 10).await)
            } else {
                None
            }
        },
        async {
            if has_prompts {
                let req = prompts_list_request(prompts_id);
                Some(send_request(stdin, pending, prompts_id, &req, 10).await)
            } else {
                None
            }
        },
    );

    // Parse results, logging errors but never panicking or propagating (Risk 3).
    let tools = parse_tools_result(server_name, tools_result);
    let resources = parse_resources_result(server_name, resources_result);
    let prompts = parse_prompts_result(server_name, prompts_result);

    McpCapabilities {
        tools,
        resources,
        prompts,
        introspected_at: Some(std::time::Instant::now()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Result parsers (graceful degradation)
// ─────────────────────────────────────────────────────────────────────────────

/// Parse the `tools/list` response into a `Vec<McpTool>`.
///
/// Returns an empty `Vec` (with a warning) on any failure path:
/// - `None` → capability not declared by the server (skipped, no warning).
/// - `Err` → timeout or IO error.
/// - Response has `error` field → server refused the request.
/// - Deserialization failure → malformed response.
fn parse_tools_result(
    server_name: &str,
    result: Option<anyhow::Result<JsonRpcResponse>>,
) -> Vec<McpTool> {
    let response = match result {
        None => return Vec::new(), // capability not declared — silent skip
        Some(Err(e)) => {
            tracing::warn!(server = %server_name, "tools/list request failed: {e}");
            return Vec::new();
        }
        Some(Ok(r)) => r,
    };

    if let Some(err) = response.error {
        tracing::warn!(server = %server_name, "tools/list returned error: {err}");
        return Vec::new();
    }

    match response.result {
        None => {
            tracing::warn!(server = %server_name, "tools/list returned empty result");
            Vec::new()
        }
        Some(value) => match serde_json::from_value::<ToolsListResult>(value) {
            Ok(list) => list.tools,
            Err(e) => {
                tracing::warn!(server = %server_name, "Failed to parse tools/list result: {e}");
                Vec::new()
            }
        },
    }
}

/// Parse the `resources/list` response into a `Vec<McpResource>`.
///
/// Returns an empty `Vec` (with a warning) on any failure path.
fn parse_resources_result(
    server_name: &str,
    result: Option<anyhow::Result<JsonRpcResponse>>,
) -> Vec<McpResource> {
    let response = match result {
        None => return Vec::new(), // capability not declared — silent skip
        Some(Err(e)) => {
            tracing::warn!(server = %server_name, "resources/list request failed: {e}");
            return Vec::new();
        }
        Some(Ok(r)) => r,
    };

    if let Some(err) = response.error {
        tracing::warn!(server = %server_name, "resources/list returned error: {err}");
        return Vec::new();
    }

    match response.result {
        None => {
            tracing::warn!(server = %server_name, "resources/list returned empty result");
            Vec::new()
        }
        Some(value) => match serde_json::from_value::<ResourcesListResult>(value) {
            Ok(list) => list.resources,
            Err(e) => {
                tracing::warn!(
                    server = %server_name,
                    "Failed to parse resources/list result: {e}"
                );
                Vec::new()
            }
        },
    }
}

/// Parse the `prompts/list` response into a `Vec<McpPrompt>`.
///
/// Returns an empty `Vec` (with a warning) on any failure path.
fn parse_prompts_result(
    server_name: &str,
    result: Option<anyhow::Result<JsonRpcResponse>>,
) -> Vec<McpPrompt> {
    let response = match result {
        None => return Vec::new(), // capability not declared — silent skip
        Some(Err(e)) => {
            tracing::warn!(server = %server_name, "prompts/list request failed: {e}");
            return Vec::new();
        }
        Some(Ok(r)) => r,
    };

    if let Some(err) = response.error {
        tracing::warn!(server = %server_name, "prompts/list returned error: {err}");
        return Vec::new();
    }

    match response.result {
        None => {
            tracing::warn!(server = %server_name, "prompts/list returned empty result");
            Vec::new()
        }
        Some(value) => match serde_json::from_value::<PromptsListResult>(value) {
            Ok(list) => list.prompts,
            Err(e) => {
                tracing::warn!(
                    server = %server_name,
                    "Failed to parse prompts/list result: {e}"
                );
                Vec::new()
            }
        },
    }
}

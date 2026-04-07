//! ClawEnv MCP Bridge Server
//!
//! An MCP (Model Context Protocol) server that exposes host machine
//! capabilities to AI agents running inside ClawEnv sandboxes.
//!
//! This server communicates via stdio (stdin/stdout) using the MCP protocol,
//! and proxies requests to the ClawEnv Bridge HTTP API running on the host.
//!
//! ## Usage (inside sandbox)
//!
//! ```bash
//! # The MCP server runs inside the sandbox, connects to Bridge on host
//! clawenv-mcp-bridge --bridge-url http://host.lima.internal:3100
//! ```
//!
//! ## MCP Tools Provided
//!
//! - `file_read` — Read a file on the host
//! - `file_write` — Write a file on the host
//! - `file_list` — List directory contents on the host
//! - `exec` — Execute a command on the host
//! - `browser_open` — Open a URL in the host browser

mod tools;

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

#[derive(Parser)]
#[command(name = "clawenv-mcp-bridge", about = "MCP Server for ClawEnv Bridge API")]
struct Args {
    /// Bridge API base URL (host machine)
    #[arg(long, default_value = "http://host.lima.internal:3100")]
    bridge_url: String,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
}

/// MCP JSON-RPC request
#[derive(Deserialize, Debug)]
struct McpRequest {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// MCP JSON-RPC response
#[derive(Serialize)]
struct McpResponse {
    jsonrpc: String,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpError>,
}

#[derive(Serialize)]
struct McpError {
    code: i32,
    message: String,
}

/// Tool definition for MCP
#[derive(Serialize)]
struct ToolDef {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: serde_json::Value,
}

fn tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "file_read".into(),
            description: "Read a file from the host machine".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path (supports ~/ for home)" }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "file_write".into(),
            description: "Write content to a file on the host machine".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "content": { "type": "string", "description": "File content" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDef {
            name: "file_list".into(),
            description: "List directory contents on the host machine".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path" }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "exec".into(),
            description: "Execute a command on the host machine".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command to execute" },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Command arguments" }
                },
                "required": ["command"]
            }),
        },
        ToolDef {
            name: "browser_open".into(),
            description: "Open a URL in the host's default browser".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to open" }
                },
                "required": ["url"]
            }),
        },
    ]
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(&args.log_level)
        .with_writer(io::stderr) // MCP uses stdout for protocol, logs go to stderr
        .init();

    tracing::info!("ClawEnv MCP Bridge starting, bridge_url={}", args.bridge_url);

    let client = tools::BridgeClient::new(&args.bridge_url);

    // MCP stdio loop: read JSON-RPC from stdin, write responses to stdout
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: McpRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Invalid JSON-RPC: {e}");
                continue;
            }
        };

        let response = handle_request(&client, &request).await;

        if let Some(resp) = response {
            let json = serde_json::to_string(&resp).unwrap_or_default();
            writeln!(stdout, "{json}").ok();
            stdout.flush().ok();
        }
    }
}

async fn handle_request(client: &tools::BridgeClient, req: &McpRequest) -> Option<McpResponse> {
    let id = req.id.clone().unwrap_or(serde_json::Value::Null);

    match req.method.as_str() {
        "initialize" => Some(McpResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "clawenv-mcp-bridge",
                    "version": "0.1.0"
                }
            })),
            error: None,
        }),

        "tools/list" => Some(McpResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::json!({
                "tools": tool_definitions()
            })),
            error: None,
        }),

        "tools/call" => {
            let tool_name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = req.params.get("arguments").cloned().unwrap_or_default();

            let result = client.call_tool(tool_name, &arguments).await;

            match result {
                Ok(content) => Some(McpResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: Some(serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": content
                        }]
                    })),
                    error: None,
                }),
                Err(e) => Some(McpResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: None,
                    error: Some(McpError {
                        code: -32000,
                        message: e.to_string(),
                    }),
                }),
            }
        }

        "notifications/initialized" | "ping" => {
            if req.method == "ping" {
                Some(McpResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: Some(serde_json::json!({})),
                    error: None,
                })
            } else {
                None // notifications don't need response
            }
        }

        _ => Some(McpResponse {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(McpError {
                code: -32601,
                message: format!("Method not found: {}", req.method),
            }),
        }),
    }
}

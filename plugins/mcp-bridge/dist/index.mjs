#!/usr/bin/env node
/**
 * ClawEnv MCP Bridge — MCP Server for host machine access
 *
 * Runs inside the sandbox (Lima/Podman/WSL2) as an MCP stdio server.
 * Proxies tool calls to the ClawEnv Bridge HTTP API on the host.
 *
 * Usage:
 *   node index.mjs                                    # default bridge URL
 *   node index.mjs --bridge-url http://host.lima.internal:3100
 *
 * Register with OpenClaw:
 *   openclaw mcp set clawenv '{"command":"node","args":["/workspace/mcp-bridge/dist/index.mjs"]}'
 */
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
// Parse CLI args
const args = process.argv.slice(2);
let bridgeUrl = "http://host.lima.internal:3100";
for (let i = 0; i < args.length; i++) {
    if (args[i] === "--bridge-url" && args[i + 1]) {
        bridgeUrl = args[i + 1];
        i++;
    }
}
// Auto-detect bridge URL by platform
if (bridgeUrl === "auto") {
    try {
        // Podman
        const resp = await fetch("http://host.containers.internal:3100/api/health");
        if (resp.ok)
            bridgeUrl = "http://host.containers.internal:3100";
    }
    catch {
        // fallback to Lima
        bridgeUrl = "http://host.lima.internal:3100";
    }
}
console.error(`[clawenv-mcp] Bridge URL: ${bridgeUrl}`);
// Helper: call Bridge API
async function bridgeCall(endpoint, body) {
    const url = `${bridgeUrl}${endpoint}`;
    const resp = await fetch(url, {
        method: body ? "POST" : "GET",
        headers: body ? { "Content-Type": "application/json" } : {},
        body: body ? JSON.stringify(body) : undefined,
    });
    if (!resp.ok) {
        const text = await resp.text();
        throw new Error(`Bridge API error (${resp.status}): ${text}`);
    }
    return resp.json();
}
// Create MCP Server
const server = new McpServer({
    name: "clawenv-bridge",
    version: "0.1.0",
});
// === Tool: file_read ===
server.tool("file_read", "Read a file from the host machine. Path supports ~/ for home directory.", {
    path: z.string().describe("File path on the host (e.g. ~/Documents/notes.txt)"),
}, async ({ path }) => {
    const data = await bridgeCall("/api/file/read", { path });
    return {
        content: [{ type: "text", text: data.content ?? JSON.stringify(data) }],
    };
});
// === Tool: file_write ===
server.tool("file_write", "Write content to a file on the host machine. Requires permission.", {
    path: z.string().describe("File path on the host"),
    content: z.string().describe("Content to write"),
}, async ({ path, content }) => {
    const data = await bridgeCall("/api/file/write", { path, content });
    return {
        content: [
            { type: "text", text: data.ok ? `Written ${content.length} bytes to ${path}` : `Failed: ${JSON.stringify(data)}` },
        ],
    };
});
// === Tool: file_list ===
server.tool("file_list", "List contents of a directory on the host machine.", {
    path: z.string().describe("Directory path on the host (e.g. ~/Projects)"),
}, async ({ path }) => {
    const data = await bridgeCall("/api/file/list", { path });
    const entries = data.entries ?? [];
    const formatted = entries
        .map((e) => `${e.is_dir ? "📁" : "📄"} ${e.name}${e.is_dir ? "/" : ` (${e.size} bytes)`}`)
        .join("\n");
    return {
        content: [{ type: "text", text: formatted || "Empty directory" }],
    };
});
// === Tool: exec ===
server.tool("exec", "Execute a command on the host machine. The command must be in the Bridge Server's allow list.", {
    command: z.string().describe("Command to execute (e.g. git, npm, python3)"),
    args: z.array(z.string()).optional().describe("Command arguments"),
}, async ({ command, args }) => {
    const data = await bridgeCall("/api/exec", { command, args: args ?? [] });
    let output = data.stdout ?? "";
    if (data.stderr)
        output += `\n[stderr] ${data.stderr}`;
    if (data.exit_code !== 0)
        output += `\n[exit code: ${data.exit_code}]`;
    return {
        content: [{ type: "text", text: output || "(no output)" }],
    };
});
// === Tool: host_info ===
server.tool("host_info", "Get information about the host machine and Bridge Server status.", {}, async () => {
    const data = await bridgeCall("/api/health");
    const perms = await bridgeCall("/api/permissions");
    return {
        content: [
            {
                type: "text",
                text: `Host Bridge Server:\n${JSON.stringify(data, null, 2)}\n\nPermissions:\n${JSON.stringify(perms, null, 2)}`,
            },
        ],
    };
});
// Start stdio transport
const transport = new StdioServerTransport();
await server.connect(transport);
console.error("[clawenv-mcp] Server started on stdio");

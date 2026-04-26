#!/usr/bin/env node
/**
 * ClawEnv hw-notify — MCP Server for hardware device notifications
 *
 * Runs inside the sandbox (or native) as an MCP stdio server.
 * Agent calls notify() → this plugin POSTs to Bridge /api/hw/notify.
 *
 * Usage:
 *   node hw-notify.mjs
 *   node hw-notify.mjs --bridge-url http://host.lima.internal:3100
 *
 * Register with OpenClaw:
 *   openclaw mcp set hw-notify '{"command":"node","args":["/workspace/hw-notify/index.mjs"]}'
 */
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

// Parse CLI args
const args = process.argv.slice(2);
let bridgeUrl = "";
for (let i = 0; i < args.length; i++) {
    if (args[i] === "--bridge-url" && args[i + 1]) {
        bridgeUrl = args[i + 1];
        i++;
    }
}

// Auto-detect bridge URL (same logic as mcp-bridge)
if (!bridgeUrl) {
    const hostIp = process.env.CLAWENV_HOST_IP;
    if (hostIp) {
        bridgeUrl = `http://${hostIp}:3100`;
    } else {
        try {
            const resp = await fetch("http://host.containers.internal:3100/api/health", { signal: AbortSignal.timeout(2000) });
            if (resp.ok) bridgeUrl = "http://host.containers.internal:3100";
        } catch { /* not Podman */ }
        if (!bridgeUrl) {
            bridgeUrl = "http://host.lima.internal:3100";
        }
    }
}
console.error(`[hw-notify] Bridge URL: ${bridgeUrl}`);

const server = new McpServer({ name: "hw-notify", version: "0.1.0" });

server.tool(
    "notify",
    "Send a notification to connected hardware devices. Use this when you need to alert the user on their hardware device (e.g., task completed, action required, important update).",
    {
        message: z.string().describe("Notification message content"),
        level: z.enum(["info", "alert", "action"]).optional().describe("Urgency level: info (default), alert, or action (requires user response)"),
        device_id: z.string().optional().describe("Target device ID, omit or '*' to broadcast to all devices"),
    },
    async ({ message, level, device_id }) => {
        const resp = await fetch(`${bridgeUrl}/api/hw/notify`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
                message,
                level: level || "info",
                device_id: device_id || "*",
                from_instance: process.env.CLAWENV_INSTANCE || "",
            }),
        });
        if (!resp.ok) {
            const text = await resp.text();
            return { content: [{ type: "text", text: `Notification failed (${resp.status}): ${text}` }] };
        }
        return { content: [{ type: "text", text: `Notification sent: ${message}` }] };
    }
);

const transport = new StdioServerTransport();
await server.connect(transport);
console.error("[hw-notify] Server started on stdio");

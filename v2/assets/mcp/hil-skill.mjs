#!/usr/bin/env node
/**
 * ClawEnv HIL Skill — Human-in-the-Loop browser intervention via MCP
 *
 * Runs inside the sandbox as an MCP stdio server.
 * When an agent encounters a CAPTCHA, 2FA, or other browser challenge that
 * requires human intervention, it calls the `hil_request` tool.
 *
 * Flow:
 *   1. Agent calls hil_request(reason)
 *   2. This skill POSTs to ClawEnv Bridge /api/hil/request
 *   3. ClawEnv switches browser to interactive (noVNC) mode
 *   4. User resolves the challenge and clicks "Continue Auto"
 *   5. ClawEnv responds to the HTTP request
 *   6. This skill returns success to the agent
 *   7. Agent continues automation
 *
 * Register with OpenClaw:
 *   openclaw mcp set clawenv-hil '{"command":"node","args":["/workspace/hil-skill/index.mjs"]}'
 */
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

// Parse CLI args
const args = process.argv.slice(2);
let bridgeUrl = "";
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--bridge-url" && args[i + 1]) {
    bridgeUrl = args[i + 1]; i++;
  }
}

// Auto-detect bridge URL
if (!bridgeUrl) {
  const hostIp = process.env.CLAWENV_HOST_IP;
  if (hostIp) {
    bridgeUrl = `http://${hostIp}:3100`;
  } else {
    try {
      const resp = await fetch("http://host.containers.internal:3100/api/health", { signal: AbortSignal.timeout(2000) });
      if (resp.ok) bridgeUrl = "http://host.containers.internal:3100";
    } catch { /* not Podman */ }
    if (!bridgeUrl) bridgeUrl = "http://host.lima.internal:3100";
  }
}

console.error(`[clawenv-hil] Bridge URL: ${bridgeUrl}`);

const server = new McpServer({
  name: "clawenv-hil",
  version: "0.1.0",
});

// === Tool: hil_request ===
// Blocks until human completes the intervention
server.tool(
  "hil_request",
  "Request human intervention for a browser task. " +
  "Use this when the browser encounters a CAPTCHA, 2FA prompt, login wall, " +
  "cookie consent, or any challenge that cannot be automated. " +
  "This will open the browser in interactive mode for the user. " +
  "The call blocks until the user finishes and clicks 'Continue Auto'.",
  {
    reason: z.string().describe(
      "Why human help is needed (e.g. 'CAPTCHA detected on login page', '2FA code required')"
    ),
    url: z.string().optional().describe(
      "The URL where intervention is needed (informational, shown to user)"
    ),
  },
  async ({ reason, url }) => {
    console.error(`[clawenv-hil] HIL requested: ${reason}${url ? ` at ${url}` : ""}`);

    try {
      // This POST blocks until the user completes intervention
      const resp = await fetch(`${bridgeUrl}/api/hil/request`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ reason, url: url ?? "" }),
        signal: AbortSignal.timeout(600_000), // 10 min max wait
      });

      if (!resp.ok) {
        const text = await resp.text();
        return {
          content: [{ type: "text", text: `HIL request failed: ${text}` }],
          isError: true,
        };
      }

      const data = await resp.json();
      console.error(`[clawenv-hil] HIL completed: ${JSON.stringify(data)}`);

      return {
        content: [{
          type: "text",
          text: `Human intervention completed. ${data.message ?? "User finished the task."}` +
                `${data.notes ? `\nUser notes: ${data.notes}` : ""}`,
        }],
      };
    } catch (err) {
      return {
        content: [{ type: "text", text: `HIL request error: ${err.message}` }],
        isError: true,
      };
    }
  }
);

// === Tool: hil_status ===
// Check if HIL is currently active
server.tool(
  "hil_status",
  "Check if a human intervention session is currently active.",
  {},
  async () => {
    try {
      const resp = await fetch(`${bridgeUrl}/api/hil/status`, { signal: AbortSignal.timeout(5000) });
      const data = await resp.json();
      return {
        content: [{ type: "text", text: JSON.stringify(data, null, 2) }],
      };
    } catch (err) {
      return {
        content: [{ type: "text", text: `Cannot check HIL status: ${err.message}` }],
        isError: true,
      };
    }
  }
);

const transport = new StdioServerTransport();
await server.connect(transport);
console.error("[clawenv-hil] HIL skill server started on stdio");

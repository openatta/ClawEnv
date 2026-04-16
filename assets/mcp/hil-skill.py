#!/usr/bin/env python3
"""
ClawEnv HIL Skill — Human-in-the-Loop browser intervention via MCP (Python version)

Runs inside the sandbox as an MCP stdio server.
When an agent encounters a CAPTCHA, 2FA, or other browser challenge that
requires human intervention, it calls the `hil_request` tool.

Flow:
  1. Agent calls hil_request(reason)
  2. This skill POSTs to ClawEnv Bridge /api/hil/request
  3. ClawEnv switches browser to interactive (noVNC) mode
  4. User resolves the challenge and clicks "Continue Auto"
  5. ClawEnv responds to the HTTP request
  6. This skill returns success to the agent
  7. Agent continues automation

Register with Hermes Agent:
  hermes mcp add clawenv-hil --config '{"command":"python3","args":["/workspace/hil-skill/skill.py"]}'
"""
from __future__ import annotations

import sys
import os
import json
import argparse
import asyncio

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent

# Parse CLI args
parser = argparse.ArgumentParser()
parser.add_argument("--bridge-url", default="")
args = parser.parse_args()

bridge_url = args.bridge_url

# Auto-detect bridge URL
if not bridge_url:
    host_ip = os.environ.get("CLAWENV_HOST_IP")
    if host_ip:
        bridge_url = f"http://{host_ip}:3100"
    else:
        try:
            r = httpx.get("http://host.containers.internal:3100/api/health", timeout=2)
            if r.status_code == 200:
                bridge_url = "http://host.containers.internal:3100"
        except Exception:
            pass
        if not bridge_url:
            bridge_url = "http://host.lima.internal:3100"

print(f"[clawenv-hil] Bridge URL: {bridge_url}", file=sys.stderr)

# HTTP client with 10-min timeout for HIL blocking requests
client = httpx.AsyncClient(base_url=bridge_url, timeout=600)


def text(s: str) -> list:
    """Wrap a string in MCP TextContent list for tool responses."""
    return [TextContent(type="text", text=s)]


server = Server("clawenv-hil")


@server.tool()
async def hil_request(reason: str, url: str = "") -> list:
    """Request human intervention for a browser task.

    Use this when the browser encounters a CAPTCHA, 2FA prompt, login wall,
    cookie consent, or any challenge that cannot be automated.
    This will open the browser in interactive mode for the user.
    The call blocks until the user finishes and clicks 'Continue Auto'.
    """
    print(f"[clawenv-hil] HIL requested: {reason}{f' at {url}' if url else ''}", file=sys.stderr)

    try:
        resp = await client.post(
            "/api/hil/request",
            json={"reason": reason, "url": url},
        )

        if resp.status_code != 200:
            return text(f"HIL request failed: {resp.text}")

        data = resp.json()
        print(f"[clawenv-hil] HIL completed: {json.dumps(data)}", file=sys.stderr)

        result = f"Human intervention completed. {data.get('message', 'User finished the task.')}"
        if data.get("notes"):
            result += f"\nUser notes: {data['notes']}"
        return text(result)

    except Exception as e:
        return text(f"HIL request error: {e}")


@server.tool()
async def hil_status() -> list:
    """Check if a human intervention session is currently active."""
    try:
        resp = await client.get("/api/hil/status", timeout=5)
        data = resp.json()
        return text(json.dumps(data, indent=2))
    except Exception as e:
        return text(f"Cannot check HIL status: {e}")


async def main():
    async with stdio_server() as (read_stream, write_stream):
        print("[clawenv-hil] HIL skill server started on stdio", file=sys.stderr)
        await server.run(read_stream, write_stream, server.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())

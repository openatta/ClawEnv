#!/usr/bin/env python3
"""
ClawEnv hw-notify — MCP Server for hardware device notifications (Python version)

Runs inside the sandbox (or native) as an MCP stdio server.
Agent calls notify() → this plugin POSTs to Bridge /api/hw/notify.

Usage:
  python3 hw-notify.py
  python3 hw-notify.py --bridge-url http://host.lima.internal:3100

Register with Hermes Agent:
  hermes mcp add hw-notify --config '{"command":"python3","args":["/workspace/hw-notify/notify.py"]}'
"""
from __future__ import annotations

import sys
import os
import json
import argparse
import asyncio
from typing import Optional

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent

parser = argparse.ArgumentParser()
parser.add_argument("--bridge-url", default="")
args = parser.parse_args()

bridge_url = args.bridge_url

# Auto-detect bridge URL (same logic as mcp-bridge)
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

print(f"[hw-notify] Bridge URL: {bridge_url}", file=sys.stderr)

client = httpx.AsyncClient(base_url=bridge_url, timeout=10)

server = Server("hw-notify")


def text(s: str) -> list:
    return [TextContent(type="text", text=s)]


@server.tool()
async def notify(
    message: str,
    level: Optional[str] = "info",
    device_id: Optional[str] = "*",
) -> list:
    """Send a notification to connected hardware devices. Use this when you need to alert the user on their hardware device (e.g., task completed, action required, important update)."""
    resp = await client.post("/api/hw/notify", json={
        "message": message,
        "level": level or "info",
        "device_id": device_id or "*",
        "from_instance": os.environ.get("CLAWENV_INSTANCE", ""),
    })
    if resp.status_code != 200:
        return text(f"Notification failed ({resp.status_code}): {resp.text}")
    return text(f"Notification sent: {message}")


async def main():
    async with stdio_server() as (read_stream, write_stream):
        print("[hw-notify] Server started on stdio", file=sys.stderr)
        await server.run(read_stream, write_stream, server.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())

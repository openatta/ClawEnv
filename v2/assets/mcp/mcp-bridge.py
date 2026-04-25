#!/usr/bin/env python3
"""
ClawEnv MCP Bridge — MCP Server for host machine access (Python version)

Runs inside the sandbox (Lima/Podman/WSL2) as an MCP stdio server.
Proxies tool calls to the ClawEnv Bridge HTTP API on the host.

Usage:
  python3 bridge.py                                    # default bridge URL
  python3 bridge.py --bridge-url http://host.lima.internal:3100

Register with Hermes Agent:
  hermes mcp add clawenv --config '{"command":"python3","args":["/workspace/mcp-bridge/bridge.py"]}'
"""
from __future__ import annotations

import sys
import os
import json
import argparse
import asyncio
from typing import Optional, List

import httpx
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent

# Parse CLI args
parser = argparse.ArgumentParser()
parser.add_argument("--bridge-url", default="")
args = parser.parse_args()

bridge_url = args.bridge_url

# Auto-detect bridge URL if not specified via CLI
if not bridge_url:
    host_ip = os.environ.get("CLAWENV_HOST_IP")
    if host_ip:
        bridge_url = f"http://{host_ip}:3100"
    else:
        # Try Podman host alias (sync probe, safe at module init before event loop)
        try:
            r = httpx.get("http://host.containers.internal:3100/api/health", timeout=2)
            if r.status_code == 200:
                bridge_url = "http://host.containers.internal:3100"
        except Exception:
            pass
        # Fallback: Lima default
        if not bridge_url:
            bridge_url = "http://host.lima.internal:3100"

print(f"[clawenv-mcp] Bridge URL: {bridge_url}", file=sys.stderr)

# HTTP client
client = httpx.AsyncClient(base_url=bridge_url, timeout=30)


async def bridge_call(endpoint: str, body: Optional[dict] = None) -> dict:
    if body is not None:
        resp = await client.post(endpoint, json=body)
    else:
        resp = await client.get(endpoint)
    resp.raise_for_status()
    return resp.json()


def text(s: str) -> list:
    """Wrap a string in MCP TextContent list for tool responses."""
    return [TextContent(type="text", text=s)]


# Create MCP Server
server = Server("clawenv-bridge")


@server.tool()
async def file_read(path: str) -> list:
    """Read a file from the host machine. Path supports ~/ for home directory."""
    data = await bridge_call("/api/file/read", {"path": path})
    return text(data.get("content", json.dumps(data)))


@server.tool()
async def file_write(path: str, content: str) -> list:
    """Write content to a file on the host machine. Requires permission."""
    data = await bridge_call("/api/file/write", {"path": path, "content": content})
    if data.get("ok"):
        return text(f"Written {len(content)} bytes to {path}")
    return text(f"Failed: {json.dumps(data)}")


@server.tool()
async def file_list(path: str) -> list:
    """List contents of a directory on the host machine."""
    data = await bridge_call("/api/file/list", {"path": path})
    entries = data.get("entries", [])
    if not entries:
        return text("Empty directory")
    lines = []
    for e in entries:
        if e.get("is_dir"):
            lines.append(f"\U0001f4c1 {e['name']}/")
        else:
            lines.append(f"\U0001f4c4 {e['name']} ({e.get('size', 0)} bytes)")
    return text("\n".join(lines))


@server.tool()
async def exec_command(command: str, args: Optional[List[str]] = None) -> list:
    """Execute a command on the host machine. The command must be in the Bridge Server's allow list."""
    data = await bridge_call("/api/exec", {"command": command, "args": args or []})
    output = data.get("stdout", "")
    if data.get("stderr"):
        output += f"\n[stderr] {data['stderr']}"
    if data.get("exit_code", 0) != 0:
        output += f"\n[exit code: {data['exit_code']}]"
    return text(output or "(no output)")


@server.tool()
async def host_info() -> list:
    """Get information about the host machine and Bridge Server status."""
    health = await bridge_call("/api/health")
    perms = await bridge_call("/api/permissions")
    return text(
        f"Host Bridge Server:\n{json.dumps(health, indent=2)}\n\n"
        f"Permissions:\n{json.dumps(perms, indent=2)}"
    )


async def main():
    async with stdio_server() as (read_stream, write_stream):
        print("[clawenv-mcp] Server started on stdio", file=sys.stderr)
        await server.run(read_stream, write_stream, server.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())

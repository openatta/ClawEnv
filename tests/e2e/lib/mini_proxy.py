#!/usr/bin/env python3
"""
Minimal HTTP/HTTPS-CONNECT proxy that forwards to an upstream proxy.

Purpose: let the E2E test subprocess set HTTPS_PROXY to a port we control,
and have all traffic ultimately flow through the user's real local proxy
(Clash / Surge / etc). This gives the test a "mockable" proxy endpoint
without requiring squid/tinyproxy installation — pure stdlib Python.

Protocols:
  - HTTP GET / POST / ... — relays request line + headers + body upstream
  - HTTPS CONNECT tunnels — relays the CONNECT and then bidirectional pipe

The upstream is assumed to be an HTTP proxy (NOT SOCKS). If the user's
real proxy is SOCKS, they need to run it in "hybrid" mode with an HTTP
port (Clash's default mixed mode covers this).
"""

import argparse
import asyncio
import logging
import sys


async def pipe(reader, writer):
    """Copy bytes r→w until EOF. Swallow reset errors (common on close)."""
    try:
        while True:
            data = await reader.read(8192)
            if not data:
                break
            writer.write(data)
            await writer.drain()
    except (ConnectionResetError, BrokenPipeError, asyncio.IncompleteReadError):
        pass
    finally:
        try:
            writer.close()
        except Exception:
            pass


async def handle_client(client_r, client_w, upstream_host, upstream_port):
    """One client session. Reads headers, opens upstream, pipes both ways."""
    peer = client_w.get_extra_info('peername')
    try:
        # Read request line + headers (until blank line) — raw bytes, don't decode.
        buf = bytearray()
        while b"\r\n\r\n" not in buf:
            chunk = await client_r.read(4096)
            if not chunk:
                return
            buf.extend(chunk)
            # Cap to avoid pathological clients.
            if len(buf) > 65536:
                client_w.close()
                return

        header_end = buf.index(b"\r\n\r\n") + 4
        headers_raw = bytes(buf[:header_end])
        leftover = bytes(buf[header_end:])

        first_line = headers_raw.split(b"\r\n", 1)[0].decode("latin-1", errors="replace")
        logging.info(f"[mini_proxy] {peer} → {first_line}")

        # Open upstream proxy connection and forward request verbatim.
        up_r, up_w = await asyncio.open_connection(upstream_host, upstream_port)
        up_w.write(headers_raw)
        if leftover:
            up_w.write(leftover)
        await up_w.drain()

        # Pipe both directions concurrently until either side closes.
        await asyncio.gather(
            pipe(client_r, up_w),
            pipe(up_r, client_w),
            return_exceptions=True,
        )
    except OSError as e:
        logging.warning(f"[mini_proxy] {peer} upstream open failed: {e}")
        try:
            client_w.write(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
            await client_w.drain()
        except Exception:
            pass
        try:
            client_w.close()
        except Exception:
            pass
    except Exception as e:
        logging.warning(f"[mini_proxy] {peer} handle error: {e}")


async def main():
    ap = argparse.ArgumentParser(
        description="Mini HTTP/HTTPS-CONNECT proxy relay for E2E tests."
    )
    ap.add_argument("--listen-host", default="127.0.0.1",
                    help="Interface to bind (127.0.0.1 default; 0.0.0.0 to "
                         "let VMs reach it; LAN IP for specific interface).")
    ap.add_argument("--listen-port", type=int, required=True,
                    help="Port to listen on.")
    ap.add_argument("--upstream-host", default="127.0.0.1",
                    help="Upstream HTTP proxy host (default: 127.0.0.1).")
    ap.add_argument("--upstream-port", type=int, required=True,
                    help="Upstream HTTP proxy port (e.g. 7890 for Clash).")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
    )

    server = await asyncio.start_server(
        lambda r, w: handle_client(r, w, args.upstream_host, args.upstream_port),
        host=args.listen_host,
        port=args.listen_port,
    )
    print(
        f"mini_proxy listening on {args.listen_host}:{args.listen_port} → "
        f"{args.upstream_host}:{args.upstream_port}",
        flush=True,
    )
    # Expose a /ready-style signal via stdout so the caller can wait for
    # "ready" before kicking off tests.
    print("READY", flush=True)

    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        sys.exit(0)

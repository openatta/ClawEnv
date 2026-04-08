import { onMount, onCleanup, createSignal } from "solid-js";
import { Terminal } from "xterm";
import { FitAddon } from "@xterm/addon-fit";

type Props = {
  instanceName: string;
  ttydPort?: number;
  onClose: () => void;
};

/**
 * ttyd WebSocket protocol (from ttyd source):
 *
 * Handshake: first message after open = JSON string:
 *   {AuthToken: "", columns: N, rows: N}
 *
 * Message types (ASCII char prefix, NOT binary byte):
 *   "0" (0x30) = INPUT (client→server) / OUTPUT (server→client)
 *   "1" (0x31) = RESIZE_TERMINAL
 *   "2" (0x32) = PAUSE
 *   "3" (0x33) = RESUME
 *
 * SubProtocol: ["tty"]
 * BinaryType: "arraybuffer"
 */

const CMD_OUTPUT = "0".charCodeAt(0);  // 0x30
const CMD_INPUT = "0".charCodeAt(0);   // 0x30
const CMD_RESIZE = "1".charCodeAt(0);  // 0x31

export default function SandboxTerminal(props: Props) {
  let containerRef: HTMLDivElement | undefined;
  let term: Terminal | undefined;
  let ws: WebSocket | undefined;
  const [status, setStatus] = createSignal("Connecting...");
  const [error, setError] = createSignal("");

  onMount(() => {
    const port = props.ttydPort ?? 7681;
    const wsUrl = `ws://127.0.0.1:${port}/ws`;
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();

    try {
      term = new Terminal({
        theme: {
          background: "#0d1117",
          foreground: "#c9d1d9",
          cursor: "#58a6ff",
          selectionBackground: "#264f78",
        },
        fontSize: 13,
        fontFamily: "'Menlo', 'Monaco', 'Courier New', monospace",
        cursorBlink: true,
        scrollback: 5000,
      });

      const fitAddon = new FitAddon();
      term.loadAddon(fitAddon);

      if (containerRef) {
        term.open(containerRef);
        setTimeout(() => fitAddon.fit(), 50);
      }

      setStatus(`Connecting to ${wsUrl}...`);
      ws = new WebSocket(wsUrl, ["tty"]);
      ws.binaryType = "arraybuffer";

      ws.onopen = () => {
        setStatus(`Connected (localhost:${port})`);
        // ttyd handshake: first message must be JSON with AuthToken + terminal size
        if (term && ws) {
          const handshake = JSON.stringify({
            AuthToken: "",
            columns: term.cols,
            rows: term.rows,
          });
          ws.send(encoder.encode(handshake));
        }
      };

      ws.onmessage = (ev) => {
        if (!term) return;
        const data = new Uint8Array(ev.data as ArrayBuffer);
        if (data.length < 1) return;
        const cmd = data[0];
        const payload = data.slice(1);

        if (cmd === CMD_OUTPUT) {
          term.write(payload);
        }
        // cmd === 0x31: SET_WINDOW_TITLE — ignore
        // cmd === 0x32: SET_PREFERENCES — ignore
      };

      // Send keyboard input
      term.onData((input: string) => {
        if (ws && ws.readyState === WebSocket.OPEN) {
          if (typeof input === "string") {
            const encoded = encoder.encode(input);
            const msg = new Uint8Array(1 + encoded.length);
            msg[0] = CMD_INPUT;
            msg.set(encoded, 1);
            ws.send(msg);
          }
        }
      });

      // Send resize
      term.onResize(({ cols, rows }) => {
        if (ws && ws.readyState === WebSocket.OPEN) {
          const json = JSON.stringify({ columns: cols, rows: rows });
          const encoded = encoder.encode(json);
          const msg = new Uint8Array(1 + encoded.length);
          msg[0] = CMD_RESIZE;
          msg.set(encoded, 1);
          ws.send(msg);
        }
      });

      ws.onerror = () => {
        setError(`Cannot connect to terminal on port ${port}. Is the instance running?`);
        setStatus("Error");
      };

      ws.onclose = () => {
        setStatus("Disconnected");
        term?.writeln("\r\n\x1b[31mConnection closed\x1b[0m");
      };

      const observer = new ResizeObserver(() => setTimeout(() => fitAddon.fit(), 10));
      if (containerRef) observer.observe(containerRef);
      onCleanup(() => observer.disconnect());

    } catch (e) {
      setError(String(e));
    }
  });

  onCleanup(() => {
    ws?.close();
    term?.dispose();
  });

  return (
    <div class="fixed inset-0 bg-black/70 flex items-center justify-center z-50">
      <div class="bg-[#0d1117] border border-gray-700 rounded-xl w-[850px] h-[520px] flex flex-col shadow-2xl overflow-hidden">
        <div class="flex items-center justify-between px-4 py-2 bg-gray-900 border-b border-gray-700 shrink-0">
          <div class="flex items-center gap-2">
            <div class="flex gap-1.5">
              <button class="w-3 h-3 rounded-full bg-red-500 hover:bg-red-400"
                onClick={props.onClose} title="Close" />
              <div class="w-3 h-3 rounded-full bg-yellow-500" />
              <div class="w-3 h-3 rounded-full bg-green-500" />
            </div>
            <span class="text-xs text-gray-400 ml-2">
              {props.instanceName} — {status()}
            </span>
          </div>
          <button class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium text-white"
            onClick={props.onClose}>Close</button>
        </div>
        {error() && (
          <div class="px-4 py-2 bg-red-900/30 text-red-400 text-xs border-b border-red-700">
            {error()}
          </div>
        )}
        <div ref={containerRef} class="flex-1 min-h-0 p-1" />
      </div>
    </div>
  );
}

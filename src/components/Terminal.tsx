import { onMount, onCleanup, createSignal } from "solid-js";
import { Terminal } from "xterm";
import { FitAddon } from "@xterm/addon-fit";
import { AttachAddon } from "@xterm/addon-attach";

type Props = {
  instanceName: string;
  ttydPort?: number;
  onClose: () => void;
};

export default function SandboxTerminal(props: Props) {
  let containerRef: HTMLDivElement | undefined;
  let term: Terminal | undefined;
  let ws: WebSocket | undefined;
  const [status, setStatus] = createSignal("Connecting...");
  const [error, setError] = createSignal("");

  onMount(() => {
    const port = props.ttydPort ?? 7681;
    const wsUrl = `ws://127.0.0.1:${port}/ws`;

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

      // Connect via WebSocket to ttyd
      setStatus(`Connecting to ${wsUrl}...`);
      ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        setStatus("Connected");
        if (term && ws) {
          const attachAddon = new AttachAddon(ws);
          term.loadAddon(attachAddon);
        }
      };

      ws.onerror = () => {
        setError(`WebSocket error. Is ttyd running on port ${port}?`);
        setStatus("Error");
      };

      ws.onclose = () => {
        setStatus("Disconnected");
        term?.writeln("\r\n\x1b[31mConnection closed\x1b[0m");
      };

      // Resize handling
      const observer = new ResizeObserver(() => {
        setTimeout(() => fitAddon.fit(), 10);
      });
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
            onClick={props.onClose}>✕ Close</button>
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

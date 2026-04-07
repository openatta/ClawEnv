import { onMount, onCleanup, createSignal } from "solid-js";
import { Terminal } from "xterm";
import { FitAddon } from "@xterm/addon-fit";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type Props = {
  instanceName: string;
  onClose: () => void;
};

export default function SandboxTerminal(props: Props) {
  let containerRef: HTMLDivElement | undefined;
  let term: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let sessionId: string | null = null;
  let unlisten: UnlistenFn | null = null;
  const [error, setError] = createSignal("");

  onMount(async () => {
    try {
      // Create xterm instance
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
        convertEol: true,
      });

      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);

      if (containerRef) {
        term.open(containerRef);
        // Delay fit to ensure container has dimensions
        setTimeout(() => fitAddon?.fit(), 50);
      }

      // Listen for output from backend
      unlisten = await listen<{ session_id: string; data: string }>(
        "terminal-output",
        (event) => {
          if (event.payload.session_id === sessionId && term) {
            term.write(event.payload.data);
          }
        }
      );

      // Start terminal session
      sessionId = await invoke<string>("start_terminal", {
        instanceName: props.instanceName,
      });

      term.writeln(`\x1b[32mConnected to sandbox: ${props.instanceName}\x1b[0m\r\n`);

      // Forward user input to backend
      term.onData((data) => {
        if (sessionId) {
          invoke("write_terminal", { sessionId, data }).catch(() => {});
        }
      });

      // Handle resize
      const observer = new ResizeObserver(() => {
        setTimeout(() => fitAddon?.fit(), 10);
      });
      if (containerRef) observer.observe(containerRef);
      onCleanup(() => observer.disconnect());

    } catch (e) {
      setError(String(e));
      if (term) {
        term.writeln(`\x1b[31mConnection failed: ${e}\x1b[0m`);
      }
    }
  });

  onCleanup(() => {
    if (sessionId) {
      invoke("close_terminal", { sessionId }).catch(() => {});
    }
    unlisten?.();
    term?.dispose();
  });

  return (
    <div class="fixed inset-0 bg-black/70 flex items-center justify-center z-50">
      <div class="bg-[#0d1117] border border-gray-700 rounded-xl w-[850px] h-[520px] flex flex-col shadow-2xl overflow-hidden">
        {/* Header */}
        <div class="flex items-center justify-between px-4 py-2 bg-gray-900 border-b border-gray-700 shrink-0">
          <div class="flex items-center gap-2">
            <div class="flex gap-1.5">
              <button class="w-3 h-3 rounded-full bg-red-500 hover:bg-red-400"
                onClick={props.onClose} title="Close" />
              <div class="w-3 h-3 rounded-full bg-yellow-500" />
              <div class="w-3 h-3 rounded-full bg-green-500" />
            </div>
            <span class="text-xs text-gray-400 ml-2">
              {props.instanceName} — sandbox terminal
            </span>
          </div>
          <button
            class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium text-white"
            onClick={props.onClose}
          >
            ✕ Close
          </button>
        </div>

        {/* Terminal container */}
        <div ref={containerRef} class="flex-1 min-h-0" />
      </div>
    </div>
  );
}

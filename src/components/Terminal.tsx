import { onMount, onCleanup, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "xterm";
import { FitAddon } from "@xterm/addon-fit";

type Props = {
  instanceName: string;
  onClose: () => void;
};

export default function SandboxTerminal(props: Props) {
  let containerRef: HTMLDivElement | undefined;
  let sessionId: string | null = null;
  let unlisten: UnlistenFn | null = null;
  let term: Terminal | null = null;
  let fitAddon: FitAddon | null = null;
  const [error, setError] = createSignal("");
  const [status, setStatus] = createSignal("Initializing...");

  async function initTerminal() {
    console.log("[Terminal] initTerminal start");
    try {
      setStatus("Creating terminal...");
      console.log("[Terminal] creating Terminal instance");

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

      if (!containerRef) {
        setError("Container element not available");
        return;
      }

      term.open(containerRef);
      setTimeout(() => { try { fitAddon?.fit(); } catch {} }, 100);

      setStatus("Connecting to sandbox...");

      // Listen for output
      unlisten = await listen<{ session_id: string; data: string }>(
        "terminal-output",
        (event) => {
          if (event.payload.session_id === sessionId && term) {
            term.write(event.payload.data);
          }
        }
      );

      // Start session
      console.log("[Terminal] invoking start_terminal for:", props.instanceName);
      sessionId = await invoke<string>("start_terminal", {
        instanceName: props.instanceName,
      });
      console.log("[Terminal] session started:", sessionId);

      setStatus("Connected");
      // Don't write anything to terminal — let SSH PTY handle all output

      // Forward user input
      term.onData((data: string) => {
        if (sessionId) {
          invoke("write_terminal", { sessionId, data }).catch(() => {});
        }
      });

      // Resize
      const observer = new ResizeObserver(() => {
        setTimeout(() => { try { fitAddon?.fit(); } catch {} }, 10);
      });
      observer.observe(containerRef);
      onCleanup(() => observer.disconnect());

    } catch (e) {
      const msg = String(e);
      setError(msg);
      setStatus("Failed");
      console.error("Terminal init error:", e);
    }
  }

  onMount(() => {
    console.log("[Terminal] onMount, instanceName:", props.instanceName);
    setTimeout(() => {
      console.log("[Terminal] calling initTerminal, containerRef:", !!containerRef);
      initTerminal();
    }, 100);
  });

  onCleanup(() => {
    if (sessionId) {
      invoke("close_terminal", { sessionId }).catch(() => {});
    }
    unlisten?.();
    try { term?.dispose(); } catch {}
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
              {props.instanceName} — {status()}
            </span>
          </div>
          <button
            class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium text-white"
            onClick={props.onClose}
          >
            ✕ Close
          </button>
        </div>

        {/* Error display */}
        {error() && (
          <div class="px-4 py-2 bg-red-900/30 text-red-400 text-xs border-b border-red-700">
            Error: {error()}
          </div>
        )}

        {/* Terminal container */}
        <div ref={containerRef} class="flex-1 min-h-0 p-1" />
      </div>
    </div>
  );
}

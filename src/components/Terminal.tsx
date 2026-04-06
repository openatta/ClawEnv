import { onMount, onCleanup } from "solid-js";
import { Terminal } from "xterm";
import { FitAddon } from "@xterm/addon-fit";
import "xterm/css/xterm.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Props = {
  instanceName: string;
  onClose: () => void;
};

export default function SandboxTerminal(props: Props) {
  let containerRef: HTMLDivElement | undefined;
  let term: Terminal | null = null;
  let fitAddon: FitAddon | null = null;
  let sessionId: string | null = null;
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    // Create terminal
    term = new Terminal({
      theme: {
        background: "#0d1117",
        foreground: "#c9d1d9",
        cursor: "#58a6ff",
      },
      fontSize: 13,
      fontFamily: "Menlo, Monaco, monospace",
      cursorBlink: true,
    });
    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerRef!);
    fitAddon.fit();

    // Listen for terminal output from backend
    unlisten = (await listen<{ session_id: string; data: string }>(
      "terminal-output",
      (event) => {
        if (event.payload.session_id === sessionId && term) {
          term.write(event.payload.data);
        }
      }
    )) as unknown as () => void;

    // Start terminal session
    try {
      sessionId = await invoke<string>("start_terminal", {
        instanceName: props.instanceName,
      });
      term.write(`\r\nConnected to sandbox: ${props.instanceName}\r\n\r\n`);
    } catch (e) {
      term.write(`\r\nFailed to connect: ${e}\r\n`);
    }

    // Send user input to backend
    term.onData((data) => {
      if (sessionId) {
        invoke("write_terminal", { sessionId, data }).catch(() => {});
      }
    });

    // Handle resize
    const resizeObserver = new ResizeObserver(() => {
      fitAddon?.fit();
    });
    if (containerRef) resizeObserver.observe(containerRef);
    onCleanup(() => resizeObserver.disconnect());
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
      <div class="bg-gray-900 border border-gray-700 rounded-xl w-[800px] h-[500px] flex flex-col shadow-2xl">
        <div class="flex items-center justify-between px-4 py-2 border-b border-gray-700 shrink-0">
          <span class="text-sm font-medium text-gray-300">
            Terminal: {props.instanceName}
          </span>
          <button
            class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium text-white"
            onClick={props.onClose}
          >
            Close
          </button>
        </div>
        <div ref={containerRef} class="flex-1 p-1" />
      </div>
    </div>
  );
}

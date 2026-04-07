import { createSignal, onMount, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Props = {
  instanceName: string;
  onClose: () => void;
};

export default function SandboxTerminal(props: Props) {
  let containerRef: HTMLDivElement | undefined;
  const [connected, setConnected] = createSignal(false);
  const [error, setError] = createSignal("");
  const [lines, setLines] = createSignal<string[]>([]);
  const [inputValue, setInputValue] = createSignal("");
  let sessionId: string | null = null;
  let inputRef: HTMLInputElement | undefined;
  let outputRef: HTMLDivElement | undefined;

  // Auto-scroll
  function scrollToBottom() {
    if (outputRef) outputRef.scrollTop = outputRef.scrollHeight;
  }

  onMount(async () => {
    try {
      // Listen for output
      const unlisten = await listen<{ session_id: string; data: string }>(
        "terminal-output",
        (event) => {
          if (event.payload.session_id === sessionId) {
            // Split data into lines and append
            const newLines = event.payload.data.split('\n');
            setLines((prev) => [...prev, ...newLines].slice(-500)); // Keep last 500 lines
            setTimeout(scrollToBottom, 10);
          }
        }
      );
      onCleanup(unlisten);

      // Start session
      sessionId = await invoke<string>("start_terminal", {
        instanceName: props.instanceName,
      });
      setConnected(true);
      setLines(["Connected to sandbox: " + props.instanceName, ""]);
      inputRef?.focus();
    } catch (e) {
      setError(String(e));
    }
  });

  onCleanup(() => {
    if (sessionId) {
      invoke("close_terminal", { sessionId }).catch(() => {});
    }
  });

  async function sendCommand() {
    const cmd = inputValue();
    if (!cmd.trim() || !sessionId) return;
    setInputValue("");
    setLines((prev) => [...prev, `$ ${cmd}`]);
    try {
      await invoke("write_terminal", { sessionId, data: cmd + "\n" });
    } catch (e) {
      setLines((prev) => [...prev, `Error: ${e}`]);
    }
    setTimeout(scrollToBottom, 50);
  }

  return (
    <div class="fixed inset-0 bg-black/70 flex items-center justify-center z-50">
      <div class="bg-gray-900 border border-gray-700 rounded-xl w-[800px] h-[520px] flex flex-col shadow-2xl">
        {/* Header */}
        <div class="flex items-center justify-between px-4 py-2 border-b border-gray-700 shrink-0">
          <div class="flex items-center gap-2">
            <div class={`w-2 h-2 rounded-full ${connected() ? "bg-green-500" : "bg-red-500"}`} />
            <span class="text-sm font-medium text-gray-300">
              Terminal: {props.instanceName}
            </span>
          </div>
          <button
            class="px-3 py-0.5 text-xs bg-red-700 hover:bg-red-600 rounded font-medium text-white"
            onClick={props.onClose}
          >
            ✕ Close
          </button>
        </div>

        {/* Output area */}
        <div
          ref={outputRef}
          class="flex-1 overflow-y-auto p-3 font-mono text-xs text-green-400 bg-black min-h-0"
          onClick={() => inputRef?.focus()}
        >
          <Show when={error()}>
            <div class="text-red-400 mb-2">Error: {error()}</div>
          </Show>
          {lines().map((line) => (
            <div class={line.startsWith("$") ? "text-gray-300" : line.startsWith("Error") ? "text-red-400" : "text-green-400"}>
              {line || "\u00A0"}
            </div>
          ))}
        </div>

        {/* Input area */}
        <div class="flex items-center border-t border-gray-700 px-3 py-2 shrink-0 bg-gray-950">
          <span class="text-green-400 font-mono text-xs mr-2">$</span>
          <input
            ref={inputRef}
            type="text"
            class="flex-1 bg-transparent text-green-400 font-mono text-xs outline-none"
            placeholder={connected() ? "Type command and press Enter..." : "Connecting..."}
            disabled={!connected()}
            value={inputValue()}
            onInput={(e) => setInputValue(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") sendCommand();
            }}
          />
        </div>
      </div>
    </div>
  );
}

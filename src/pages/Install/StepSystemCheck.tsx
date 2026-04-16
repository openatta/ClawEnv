import { createSignal, Show, For, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { SystemCheckInfo } from "./types";
import LogBox from "./LogBox";

export default function StepSystemCheck(props: { onReady?: () => void }) {
  const [sysCheck, setSysCheck] = createSignal<SystemCheckInfo | null>(null);
  const [sysCheckLog, setSysCheckLog] = createSignal<string[]>([]);
  const [checking, setChecking] = createSignal(false);

  async function runSystemCheck() {
    setChecking(true);
    setSysCheckLog(["Detecting platform..."]);
    try {
      setSysCheckLog(l => [...l, "Checking OS, memory, disk..."]);
      const info = await invoke<SystemCheckInfo>("system_check");
      setSysCheck(info);
      for (const c of info.checks) {
        setSysCheckLog(l => [...l, `${c.ok ? "✓" : "✗"} ${c.name}: ${c.detail}`]);
      }
      setSysCheckLog(l => [...l, "System check complete."]);
      props.onReady?.();
    } catch (e) {
      setSysCheckLog(l => [...l, `ERROR: ${e}`]);
    } finally {
      setChecking(false);
    }
  }

  onMount(() => {
    if (!sysCheck()) runSystemCheck();
  });

  return (
    <div>
      <h2 class="text-xl font-bold mb-3">System Check</h2>
      <Show when={sysCheck()}>
        <div class="space-y-1.5 mb-3">
          <For each={sysCheck()!.checks}>
            {(c) => (
              <div class={`flex items-center gap-2 text-sm ${
                c.ok ? "text-green-400" : c.info_only ? "text-gray-400" : "text-red-400"
              }`}>
                <span>{c.ok ? "✓" : c.info_only ? "○" : "✗"}</span>
                <span class="w-36 text-gray-300">{c.name}</span>
                <span>{c.detail}</span>
              </div>
            )}
          </For>
        </div>
      </Show>
      <LogBox logs={sysCheckLog()} />
      <Show when={!checking() && sysCheck()}>
        <button class="mt-2 px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded" onClick={runSystemCheck}>
          Re-check
        </button>
      </Show>
    </div>
  );
}

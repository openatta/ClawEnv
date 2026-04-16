import { createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

export default function StepApiKey(props: {
  apiKey: () => string;
  onApiKeyChange: (k: string) => void;
  clawDisplayName: string;
}) {
  const [apiKeyTesting, setApiKeyTesting] = createSignal(false);
  const [apiKeyResult, setApiKeyResult] = createSignal<{ ok: boolean; msg: string } | null>(null);

  async function testApiKey() {
    setApiKeyTesting(true);
    setApiKeyResult(null);
    try {
      const msg = await invoke<string>("test_api_key", { apiKey: props.apiKey() });
      setApiKeyResult({ ok: true, msg });
    } catch (e) {
      setApiKeyResult({ ok: false, msg: String(e) });
    } finally {
      setApiKeyTesting(false);
    }
  }

  return (
    <div>
      <h2 class="text-xl font-bold mb-3">API Key</h2>
      <p class="text-sm text-gray-400 mb-3">Enter your {props.clawDisplayName} API key. Stored securely in system keychain.</p>
      <div class="flex gap-2 items-center mb-2">
        <input type="password" placeholder="sk-..." value={props.apiKey()} onInput={e => props.onApiKeyChange(e.currentTarget.value)}
          class="bg-gray-800 border border-gray-600 rounded px-3 py-2 w-80 text-sm" />
        <button class="px-3 py-2 text-sm bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
          disabled={apiKeyTesting() || !props.apiKey()} onClick={testApiKey}>
          {apiKeyTesting() ? "..." : "Test"}
        </button>
      </div>
      <Show when={apiKeyResult()}>
        <div class={`text-sm ${apiKeyResult()!.ok ? "text-green-400" : "text-red-400"}`}>
          {apiKeyResult()!.ok ? "✓" : "✗"} {apiKeyResult()!.msg}
        </div>
      </Show>
      <p class="text-xs text-gray-500 mt-3">You can skip this and configure it later in Settings.</p>
    </div>
  );
}

import { createSignal, createEffect, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SystemProxy, ConnTestResult } from "./types";
import LogBox from "./LogBox";

export default function StepNetwork(props: {
  /** Notifies parent whenever the user's proxy selection changes. Parent
   *  stores it in InstallState so StepProgress can pass it to the install
   *  IPC. Receives the already-serialized JSON (or null = "no proxy"). */
  onProxyChange?: (proxyJson: string | null) => void;
}) {
  const [systemProxy, setSystemProxy] = createSignal<SystemProxy | null>(null);
  const [proxyMode, setProxyMode] = createSignal<"system" | "custom" | "none">("system");
  const [httpProxy, setHttpProxy] = createSignal("");
  const [httpsProxy, setHttpsProxy] = createSignal("");
  const [connTesting, setConnTesting] = createSignal(false);
  const [connResults, setConnResults] = createSignal<ConnTestResult[]>([]);
  const [connLog, setConnLog] = createSignal<string[]>([]);

  let unlistenConnStep: UnlistenFn | null = null;
  onCleanup(() => { unlistenConnStep?.(); });

  async function detectProxy() {
    setConnLog(["Detecting system proxy..."]);
    try {
      const sp = await invoke<SystemProxy>("detect_system_proxy");
      setSystemProxy(sp);
      if (sp.detected) {
        setProxyMode("system");
        setHttpProxy(sp.http_proxy);
        setHttpsProxy(sp.https_proxy);
        setConnLog(l => [...l, `Found: ${sp.source}`, `  HTTP: ${sp.http_proxy}`, `  HTTPS: ${sp.https_proxy || "(same)"}`]);
      } else {
        setConnLog(l => [...l, "No system proxy detected.", "You can configure a custom proxy or use direct connection."]);
      }
    } catch (e) {
      setConnLog(l => [...l, `Detection error: ${e}`]);
    }
  }

  function getProxyJson(): string | null {
    if (proxyMode() === "none") return JSON.stringify({ enabled: false, http_proxy: "", https_proxy: "", no_proxy: "localhost,127.0.0.1", auth_required: false, auth_user: "" });
    if (proxyMode() === "system" && systemProxy()?.detected) return JSON.stringify({ enabled: true, http_proxy: systemProxy()!.http_proxy, https_proxy: systemProxy()!.https_proxy || systemProxy()!.http_proxy, no_proxy: systemProxy()!.no_proxy || "localhost,127.0.0.1", auth_required: false, auth_user: "" });
    if (proxyMode() === "custom" && httpProxy()) return JSON.stringify({ enabled: true, http_proxy: httpProxy(), https_proxy: httpsProxy() || httpProxy(), no_proxy: "localhost,127.0.0.1", auth_required: false, auth_user: "" });
    return null; // use system default
  }

  // Push the current selection up to the wizard whenever any input changes.
  // Tracks the four reactive sources getProxyJson() reads from so edits to
  // system/custom fields propagate without the user having to click "Test".
  createEffect(() => {
    void proxyMode(); void systemProxy(); void httpProxy(); void httpsProxy();
    props.onProxyChange?.(getProxyJson());
  });

  async function testConnectivity() {
    setConnTesting(true);
    setConnResults([]);
    setConnLog(l => [...l, "", `--- Testing connectivity (mode: ${proxyMode()}) ---`]);

    unlistenConnStep = await listen<{ endpoint: string; status: string; message?: string }>("conn-test-step", (ev) => {
      const d = ev.payload;
      if (d.status === "testing") {
        setConnLog(l => [...l, `Testing ${d.endpoint}...`]);
      } else {
        setConnLog(l => [...l, `  ${d.status === "ok" ? "✓" : "✗"} ${d.endpoint}: ${d.message || ""}`]);
      }
    });

    try {
      const results = await invoke<ConnTestResult[]>("test_connectivity", { proxyJson: getProxyJson() });
      setConnResults(results);
      const ok = results.filter(r => r.ok).length;
      setConnLog(l => [...l, "────────────────────────────",
        `Summary: ${ok}/${results.length} endpoints reachable`,
        ...results.map(r => `  ${r.ok ? "✓" : "✗"} ${r.endpoint}: ${r.message}`),
        ok === results.length ? "All connectivity checks passed." : "Some endpoints are unreachable. Check your proxy settings.",
      ]);
    } catch (e) {
      setConnLog(l => [...l, `Test failed: ${e}`]);
    } finally {
      setConnTesting(false);
      unlistenConnStep?.();
    }
  }

  onMount(() => { detectProxy(); });

  return (
    <div>
      <h2 class="text-xl font-bold mb-3">Network Settings</h2>

      {/* System proxy info */}
      <div class="bg-gray-800 rounded p-3 mb-3 border border-gray-700 text-sm">
        <div class="flex items-center justify-between">
          <Show when={systemProxy()}>
            {systemProxy()!.detected ? (
              <div>
                <span class="text-green-400">✓ System proxy detected</span>
                <span class="text-gray-400 ml-2">({systemProxy()!.source})</span>
                <div class="text-xs text-gray-400 mt-1 font-mono">{systemProxy()!.http_proxy}</div>
              </div>
            ) : (
              <span class="text-gray-500">No system proxy detected</span>
            )}
          </Show>
          <button class="px-2 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded shrink-0"
            onClick={detectProxy}>Re-detect</button>
        </div>
      </div>

      {/* Proxy mode */}
      <div class="space-y-1.5 mb-3 text-sm">
        <Show when={systemProxy()?.detected}>
          <label class="flex items-center gap-2 cursor-pointer">
            <input type="radio" name="pm" checked={proxyMode() === "system"} onChange={() => { setProxyMode("system"); setHttpProxy(systemProxy()!.http_proxy); }} class="w-3.5 h-3.5" />
            Use system proxy
          </label>
        </Show>
        <label class="flex items-center gap-2 cursor-pointer">
          <input type="radio" name="pm" checked={proxyMode() === "custom"} onChange={() => setProxyMode("custom")} class="w-3.5 h-3.5" />
          Custom proxy
        </label>
        <label class="flex items-center gap-2 cursor-pointer">
          <input type="radio" name="pm" checked={proxyMode() === "none"} onChange={() => setProxyMode("none")} class="w-3.5 h-3.5" />
          No proxy (direct)
        </label>
      </div>

      <Show when={proxyMode() === "custom"}>
        <div class="space-y-2 mb-3">
          <input type="text" placeholder="http://proxy:8080" value={httpProxy()} onInput={e => setHttpProxy(e.currentTarget.value)}
            class="bg-gray-800 border border-gray-600 rounded px-2 py-1.5 w-72 text-sm" />
          <input type="text" placeholder="HTTPS (optional)" value={httpsProxy()} onInput={e => setHttpsProxy(e.currentTarget.value)}
            class="bg-gray-800 border border-gray-600 rounded px-2 py-1.5 w-72 text-sm" />
        </div>
      </Show>

      <button class="px-3 py-1.5 text-sm bg-indigo-700 hover:bg-indigo-600 rounded disabled:opacity-50 mb-3"
        disabled={connTesting()} onClick={testConnectivity}>
        {connTesting() ? "Testing..." : "Test Connectivity"}
      </button>

      {/* Log output */}
      <LogBox logs={connLog()} height="h-52" />
    </div>
  );
}

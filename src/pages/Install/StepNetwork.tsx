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
  /** Notifies parent when connectivity under the *current* selection is
   *  proven (all endpoints reachable). Parent gates the "Next" button on
   *  this — v0.3.0 bars users with no working network from entering an
   *  install that would only fail halfway. Flips back to false on any
   *  proxy edit / mode switch until the next test confirms. */
  onConnectedChange?: (connected: boolean) => void;
}) {
  const [systemProxy, setSystemProxy] = createSignal<SystemProxy | null>(null);
  const [proxyMode, setProxyMode] = createSignal<"system" | "custom" | "none">("system");
  const [httpProxy, setHttpProxy] = createSignal("");
  const [httpsProxy, setHttpsProxy] = createSignal("");
  const [connTesting, setConnTesting] = createSignal(false);
  const [connResults, setConnResults] = createSignal<ConnTestResult[]>([]);
  const [connLog, setConnLog] = createSignal<string[]>([]);
  const [connected, setConnected] = createSignal(false);

  let unlistenConnStep: UnlistenFn | null = null;
  onCleanup(() => { unlistenConnStep?.(); });

  function reportConnected(v: boolean) {
    setConnected(v);
    props.onConnectedChange?.(v);
  }

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
  // Any change also invalidates the previous "connected" verdict — the user
  // must re-prove the new selection before we let them past this step.
  createEffect(() => {
    void proxyMode(); void systemProxy(); void httpProxy(); void httpsProxy();
    props.onProxyChange?.(getProxyJson());
    reportConnected(false);
  });

  async function testConnectivity() {
    if (connTesting()) return;
    setConnTesting(true);
    setConnResults([]);
    reportConnected(false);
    setConnLog(l => [...l, "", `--- Testing connectivity (mode: ${proxyMode()}) ---`]);

    // Re-subscribe fresh each run; the previous unlisten is dropped on success.
    unlistenConnStep?.();
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
      const allOk = ok === results.length && results.length > 0;
      setConnLog(l => [...l, "────────────────────────────",
        `Summary: ${ok}/${results.length} endpoints reachable`,
        ...results.map(r => `  ${r.ok ? "✓" : "✗"} ${r.endpoint}: ${r.message}`),
        allOk
          ? "全部端点可达 / All connectivity checks passed. 可以进入下一步 / You may proceed."
          : "部分端点不可达，请调整代理或网络后重试。 / Some endpoints unreachable — adjust proxy / network and retry before continuing.",
      ]);
      reportConnected(allOk);
    } catch (e) {
      setConnLog(l => [...l, `Test failed: ${e}`]);
      reportConnected(false);
    } finally {
      setConnTesting(false);
      unlistenConnStep?.();
      unlistenConnStep = null;
    }
  }

  onMount(async () => { await detectProxy(); void testConnectivity(); });

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
            <input type="radio" name="pm" checked={proxyMode() === "system"} onChange={() => { setProxyMode("system"); setHttpProxy(systemProxy()!.http_proxy); void testConnectivity(); }} class="w-3.5 h-3.5" />
            Use system proxy
          </label>
        </Show>
        <label class="flex items-center gap-2 cursor-pointer">
          <input type="radio" name="pm" checked={proxyMode() === "custom"} onChange={() => setProxyMode("custom")} class="w-3.5 h-3.5" />
          Custom proxy
        </label>
        <label class="flex items-center gap-2 cursor-pointer">
          <input type="radio" name="pm" checked={proxyMode() === "none"} onChange={() => { setProxyMode("none"); void testConnectivity(); }} class="w-3.5 h-3.5" />
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

      <div class="flex items-center gap-3 mb-3">
        <button class="px-3 py-1.5 text-sm bg-indigo-700 hover:bg-indigo-600 rounded disabled:opacity-50"
          disabled={connTesting()} onClick={testConnectivity}>
          {connTesting() ? "Testing..." : "Test Connectivity"}
        </button>
        <Show when={!connTesting() && connResults().length > 0}>
          {connected()
            ? <span class="text-xs text-green-400">✓ 可以继续 / Ready to proceed</span>
            : <span class="text-xs text-red-400">✗ 不可继续，请先解决网络问题 / Cannot proceed — fix network first</span>}
        </Show>
      </div>

      {/* Log output */}
      <LogBox logs={connLog()} height="h-52" />
    </div>
  );
}

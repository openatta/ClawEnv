import { createSignal, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../i18n";

type Mode = "inherit" | "none" | "sync-host" | "manual";

type ProxyState = {
  mode: Mode;
  http_proxy: string;
  https_proxy: string;
  no_proxy: string;
  auth_required: boolean;
  auth_user: string;
};

/**
 * Per-instance proxy modal. Three concrete modes plus "inherit" (= no
 * per-instance override; use whatever's in global settings):
 *
 * - **同步本地代理**: frontend calls detect_system_proxy, shows the result,
 *   user confirms. Backend rewrites 127.0.0.1/localhost to the sandbox's
 *   host-reachable address (host.lima.internal / host.containers.internal /
 *   WSL nameserver IP) before writing /etc/profile.d/proxy.sh.
 * - **手动配置**: free-text HTTP / HTTPS / NO_PROXY fields.
 * - **不使用代理**: clears /etc/profile.d/proxy.sh in the sandbox + npm
 *   config, forces direct connection regardless of global setting.
 *
 * Save applies immediately to the running sandbox (if any); running claws
 * need a restart to pick up new env. Modal emits `onSave` with a flag if
 * a restart is required.
 */
export default function ProxyModal(props: {
  instanceName: string;
  sandboxType: string;
  onSave: (needsRestart: boolean) => void;
  onClose: () => void;
}) {
  const [mode, setMode] = createSignal<Mode>("inherit");
  const [httpProxy, setHttpProxy] = createSignal("");
  const [httpsProxy, setHttpsProxy] = createSignal("");
  const [noProxy, setNoProxy] = createSignal("localhost,127.0.0.1");
  const [authRequired, setAuthRequired] = createSignal(false);
  const [authUser, setAuthUser] = createSignal("");
  const [authPassword, setAuthPassword] = createSignal("");
  type TestResult = { target: string; url: string; ok: boolean; http_code: string; latency_ms: number };
  const [testResults, setTestResults] = createSignal<TestResult[] | null>(null);
  const [testing, setTesting] = createSignal(false);

  async function runConnTest(group: "international" | "china" | "all") {
    setTesting(true);
    setTestResults([]);
    try {
      const intl = ["github", "npm", "openai", "anthropic"];
      const cn = ["deepseek", "qwen", "npmmirror"];
      const picks = group === "international" ? intl : group === "china" ? cn : [...intl, ...cn];
      const results = await invoke<TestResult[]>("test_instance_network", {
        name: props.instanceName,
        targets: picks,
      });
      setTestResults(results);
    } catch (e) {
      setError(String(e));
    } finally {
      setTesting(false);
    }
  }
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal("");
  const [detectedHttp, setDetectedHttp] = createSignal("");
  const [detectedNote, setDetectedNote] = createSignal("");
  const [detecting, setDetecting] = createSignal(false);

  const isNative = () => props.sandboxType?.toLowerCase() === "native";

  onMount(async () => {
    try {
      const cur = await invoke<ProxyState>("get_instance_proxy", { name: props.instanceName });
      setMode(cur.mode as Mode);
      setHttpProxy(cur.http_proxy);
      setHttpsProxy(cur.https_proxy);
      setNoProxy(cur.no_proxy || "localhost,127.0.0.1");
      setAuthRequired(cur.auth_required);
      setAuthUser(cur.auth_user);
    } catch { /* ignore — use defaults */ }
  });

  async function detectHost() {
    setDetecting(true);
    setDetectedNote("");
    try {
      const sp = await invoke<{
        detected: boolean; source: string; http_proxy: string; note?: string; pac_url?: string;
      }>("detect_system_proxy");
      if (sp.detected && sp.http_proxy) {
        setDetectedHttp(sp.http_proxy);
        setHttpProxy(sp.http_proxy);
        setHttpsProxy(sp.http_proxy);
        setDetectedNote(t(`检测到: ${sp.source}`, `Detected: ${sp.source}`));
      } else if (sp.pac_url || sp.note) {
        setDetectedHttp("");
        setDetectedNote(sp.note || t("未检测到可用代理", "No usable proxy detected"));
      } else {
        setDetectedHttp("");
        setDetectedNote(t("未检测到系统代理", "No system proxy detected"));
      }
    } catch (e) {
      setDetectedNote(String(e));
    } finally {
      setDetecting(false);
    }
  }

  async function save() {
    setSaving(true);
    setError("");
    try {
      // "inherit" maps to clearing the per-instance override. We send
      // mode="none" with empty URLs — but actually we need a distinct
      // "inherit" signal. The IPC treats empty http_proxy as "no override
      // active" already; leaving it set to "inherit" on the wire round-trips
      // cleanly.
      const effectiveMode = mode();
      const effectiveHttp = mode() === "none" || mode() === "inherit" ? "" : httpProxy();
      const effectiveHttps = mode() === "none" || mode() === "inherit" ? "" : httpsProxy();
      const result = await invoke<{ needs_restart: boolean }>("set_instance_proxy", {
        name: props.instanceName,
        mode: effectiveMode,
        httpProxy: effectiveHttp,
        httpsProxy: effectiveHttps,
        noProxy: noProxy(),
        authRequired: authRequired() && mode() === "manual",
        authUser: mode() === "manual" ? authUser() : "",
        authPassword: mode() === "manual" && authPassword() ? authPassword() : null,
      });
      props.onSave(!!result?.needs_restart);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  // Native-mode short-circuit: per-instance proxy is not supported. We
  // inherit the OS's proxy via the GUI process's env at Tauri startup —
  // the user configures the proxy once at the system level (macOS System
  // Preferences / Windows Internet Options) and every native claw picks
  // it up. Exposing a Modal that pretends otherwise would be a liar.
  if (isNative()) {
    return (
      <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
        <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-[28rem] shadow-2xl text-white">
          <h3 class="text-base font-bold mb-3">{t("代理设置", "Proxy Settings")} — {props.instanceName}</h3>
          <div class="bg-gray-900 border border-gray-700 rounded p-3 text-sm text-gray-300 mb-4">
            <p class="mb-2">
              {t(
                "Native 模式仅使用系统代理，不做单独配置。",
                "Native mode uses the OS system proxy only — no per-instance configuration."
              )}
            </p>
            <p class="text-xs text-gray-500">
              {t(
                "如需调整，请到 macOS 系统偏好设置 / Windows Internet 选项 / Clash 等工具里修改，然后重启实例让 claw 读取新代理。",
                "To change it, edit macOS System Preferences / Windows Internet Options / your Clash-like tool, then restart the instance so the claw picks up the new proxy."
              )}
            </p>
          </div>
          <div class="flex gap-2 justify-end">
            <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
              onClick={props.onClose}>{t("关闭", "Close")}</button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-[28rem] shadow-2xl text-white">
        <h3 class="text-base font-bold mb-3">{t("代理设置", "Proxy Settings")} — {props.instanceName}</h3>

        <div class="space-y-2 mb-3 text-sm">
          <label class="flex items-center gap-2 cursor-pointer">
            <input type="radio" name="pm" checked={mode() === "inherit"}
              onChange={() => setMode("inherit")} class="w-3.5 h-3.5" />
            <span>{t("继承全局设置", "Inherit global setting")}</span>
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input type="radio" name="pm" checked={mode() === "sync-host"}
              onChange={() => setMode("sync-host")} class="w-3.5 h-3.5" />
            <span>{t("同步本地代理", "Sync host proxy")}</span>
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input type="radio" name="pm" checked={mode() === "manual"}
              onChange={() => setMode("manual")} class="w-3.5 h-3.5" />
            <span>{t("手动配置", "Manual")}</span>
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input type="radio" name="pm" checked={mode() === "none"}
              onChange={() => setMode("none")} class="w-3.5 h-3.5" />
            <span>{t("不使用代理", "No proxy")}</span>
          </label>
        </div>

        {/* Sync host */}
        <Show when={mode() === "sync-host"}>
          <div class="bg-gray-900 rounded p-3 mb-3 border border-gray-700 text-xs">
            <div class="flex items-center gap-2 mb-2">
              <button class="px-2 py-1 bg-indigo-700 hover:bg-indigo-600 rounded text-xs disabled:opacity-50"
                disabled={detecting()} onClick={detectHost}>
                {detecting() ? t("检测中...", "Detecting...") : t("检测系统代理", "Detect system proxy")}
              </button>
              <Show when={detectedHttp()}>
                <span class="text-green-400 font-mono">{detectedHttp()}</span>
              </Show>
            </div>
            <Show when={detectedNote()}>
              <p class="text-gray-400 mb-2">{detectedNote()}</p>
            </Show>
            <Show when={detectedHttp()}>
              <p class="text-yellow-500">
                {t(
                  "保存时 127.0.0.1 / localhost 会自动改写为沙盒可达的宿主机地址",
                  "127.0.0.1 / localhost is auto-rewritten to the sandbox-reachable host address on save"
                )}
              </p>
            </Show>
          </div>
        </Show>

        {/* Manual */}
        <Show when={mode() === "manual"}>
          <div class="space-y-2 mb-3">
            <div>
              <label class="block text-[11px] text-gray-400 mb-0.5">HTTP</label>
              <input type="text" placeholder="http://127.0.0.1:7890"
                value={httpProxy()} onInput={e => setHttpProxy(e.currentTarget.value)}
                class="bg-gray-900 border border-gray-600 rounded px-2 py-1.5 w-full text-sm font-mono" />
            </div>
            <div>
              <label class="block text-[11px] text-gray-400 mb-0.5">HTTPS {t("(可选)", "(optional)")}</label>
              <input type="text" placeholder="http://127.0.0.1:7890"
                value={httpsProxy()} onInput={e => setHttpsProxy(e.currentTarget.value)}
                class="bg-gray-900 border border-gray-600 rounded px-2 py-1.5 w-full text-sm font-mono" />
            </div>
            <div>
              <label class="block text-[11px] text-gray-400 mb-0.5">NO_PROXY</label>
              <input type="text" placeholder="localhost,127.0.0.1,.cn"
                value={noProxy()} onInput={e => setNoProxy(e.currentTarget.value)}
                class="bg-gray-900 border border-gray-600 rounded px-2 py-1.5 w-full text-sm font-mono" />
            </div>
            <div class="pt-2 border-t border-gray-700">
              <label class="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
                <input type="checkbox" checked={authRequired()}
                  onChange={e => setAuthRequired(e.currentTarget.checked)}
                  class="w-3.5 h-3.5" />
                {t("代理需要认证", "Proxy requires authentication")}
              </label>
              <Show when={authRequired()}>
                <div class="space-y-2 mt-2">
                  <input type="text" placeholder={t("用户名", "Username")}
                    value={authUser()} onInput={e => setAuthUser(e.currentTarget.value)}
                    class="bg-gray-900 border border-gray-600 rounded px-2 py-1.5 w-full text-sm font-mono" />
                  <input type="password" placeholder={t("密码（保存到 Keychain）", "Password (stored in Keychain)")}
                    value={authPassword()} onInput={e => setAuthPassword(e.currentTarget.value)}
                    class="bg-gray-900 border border-gray-600 rounded px-2 py-1.5 w-full text-sm font-mono" />
                  <p class="text-[10px] text-gray-500">
                    {t(
                      "密码仅保存到系统 Keychain，不写入 config.toml 也不随 bundle 导出。",
                      "Password stored only in system keychain — never in config.toml, never exported."
                    )}
                  </p>
                </div>
              </Show>
            </div>
          </div>
        </Show>

        {/* Connectivity test — runs curl INSIDE the VM with current proxy.
            Shows which targets are reachable: one-glance diagnostic for
            "proxy configured but can't reach OpenAI" scenarios. */}
        <div class="pt-2 mb-3 border-t border-gray-700">
          <div class="flex items-center gap-2 mb-2">
            <span class="text-xs text-gray-400">{t("连通性测试:", "Test connectivity:")}</span>
            <button class="px-2 py-0.5 text-[11px] bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
              disabled={testing()} onClick={() => runConnTest("international")}>
              {t("国外", "Intl")}
            </button>
            <button class="px-2 py-0.5 text-[11px] bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
              disabled={testing()} onClick={() => runConnTest("china")}>
              {t("国内", "CN")}
            </button>
            <button class="px-2 py-0.5 text-[11px] bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
              disabled={testing()} onClick={() => runConnTest("all")}>
              {t("全部", "All")}
            </button>
          </div>
          <Show when={testing()}>
            <p class="text-[11px] text-indigo-400 animate-pulse">{t("测试中...", "Testing...")}</p>
          </Show>
          <Show when={testResults() && testResults()!.length > 0}>
            <div class="bg-gray-900 rounded p-2 text-[11px] font-mono max-h-32 overflow-y-auto">
              {testResults()!.map(r => (
                <div class={r.ok ? "text-green-400" : "text-red-400"}>
                  {r.ok ? "✓" : "✗"} {r.target.padEnd(10)} {r.http_code} {r.latency_ms}ms
                </div>
              ))}
            </div>
          </Show>
        </div>

        <Show when={error()}>
          <p class="text-xs text-red-400 mb-2">{error()}</p>
        </Show>
        <p class="text-xs text-yellow-500 mb-3">
          {t(
            "保存后需要重启实例，运行中的 claw 才能读到新的代理",
            "Save will require an instance restart for the running claw to pick up the new proxy"
          )}
        </p>
        <div class="flex gap-2 justify-end">
          <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
            onClick={props.onClose}>{t("取消", "Cancel")}</button>
          <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
            disabled={saving()} onClick={save}>
            {saving() ? "..." : t("保存", "Save")}
          </button>
        </div>
      </div>
    </div>
  );
}

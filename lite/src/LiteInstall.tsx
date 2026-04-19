import { createSignal, onMount, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t } from "@shared/i18n";
import type { Instance } from "@shared/types";

type PackageInfo = {
  path: string;
  filename: string;
  platform: string;
  arch: string;
  is_native: boolean;
  size_mb: number;
  compatible: boolean;
  needs_sandbox_backend: boolean;
  sandbox_backend_available: boolean;
};

type Props = { onComplete: (inst: Instance) => void };

export default function LiteInstall(props: Props) {
  const [step, setStep] = createSignal(1);
  const [packages, setPackages] = createSignal<PackageInfo[]>([]);
  const [selected, setSelected] = createSignal<PackageInfo | null>(null);
  const [scanning, setScanning] = createSignal(true);

  // Step 2: Proxy
  const [proxyMode, setProxyMode] = createSignal<"system" | "manual" | "none">("system");
  const [systemProxy, setSystemProxy] = createSignal("");
  const [manualProxy, setManualProxy] = createSignal("");

  // Step 3: Install progress
  const [installing, setInstalling] = createSignal(false);
  const [installLogs, setInstallLogs] = createSignal<string[]>([]);
  const [installError, setInstallError] = createSignal("");
  const [installDone, setInstallDone] = createSignal(false);

  onMount(async () => {
    // Scan for packages
    try {
      const pkgs = await invoke<PackageInfo[]>("lite_scan_packages");
      setPackages(pkgs);
      // Auto-select first compatible
      const compat = pkgs.filter(p => p.compatible);
      if (compat.length > 0) setSelected(compat[0]);
    } catch (e) {
      console.error("scan failed:", e);
    }
    setScanning(false);

    // Detect system proxy
    try {
      const proxy = await invoke<{ detected: boolean; http_proxy: string }>("detect_system_proxy");
      if (proxy.detected && proxy.http_proxy) setSystemProxy(proxy.http_proxy);
    } catch {}
  });

  async function doInstall() {
    const pkg = selected();
    if (!pkg) return;
    setStep(3);
    setInstalling(true);
    setInstallLogs([]);
    setInstallError("");

    const unProgress = await listen<{ stage: string; percent: number; message: string }>(
      "install-progress", (ev) => {
        const p = ev.payload;
        setInstallLogs(l => [...l, `[${p.percent}%] ${p.message}`]);
      }
    );
    const unComplete = await listen("install-complete", () => {
      setInstallDone(true);
      setInstalling(false);
      unProgress(); unComplete(); unFailed();
    });
    const unFailed = await listen<string>("install-failed", (ev) => {
      setInstallError(String(ev.payload));
      setInstalling(false);
      unProgress(); unComplete(); unFailed();
    });

    try {
      // Set proxy if sandbox
      if (!pkg.is_native && proxyMode() !== "none") {
        const proxy = proxyMode() === "manual" ? manualProxy() : systemProxy();
        if (proxy) {
          await invoke("save_settings", {
            settingsJson: JSON.stringify({
              proxy: { enabled: true, http_proxy: proxy, https_proxy: proxy, no_proxy: "localhost,127.0.0.1", auth_required: false, auth_user: "" },
            }),
          });
        }
      }

      // Lite already wrote proxy to config.toml via save_settings above, so
      // the CLI's startup inject_proxy_env picks it up — no need to also pass
      // proxyJson here. null tells the IPC "no ephemeral override".
      await invoke("install_openclaw", {
        instanceName: "default",
        clawType: "openclaw",
        clawVersion: "latest",
        apiKey: null,
        useNative: pkg.is_native,
        installBrowser: false,
        installMcpBridge: !pkg.is_native,
        gatewayPort: 0,
        image: pkg.path,
        proxyJson: null,
      });
    } catch (e) {
      setInstallError(String(e));
      setInstalling(false);
      unProgress(); unComplete(); unFailed();
    }
  }

  async function enterMain() {
    try {
      const list = await invoke<Instance[]>("list_instances");
      if (list.length > 0) props.onComplete(list[0]);
    } catch {}
  }

  return (
    <div class="flex h-full items-center justify-center p-6">
      <div class="w-full max-w-lg">
        <h1 class="text-2xl font-bold mb-1 text-center">ClawEnv Lite</h1>
        <p class="text-sm text-gray-400 text-center mb-6">
          {t("离线安装向导", "Offline Installer")}
        </p>

        {/* Step 1: Package Selection */}
        <Show when={step() === 1}>
          <div class="bg-gray-800 rounded-xl p-5 border border-gray-700">
            <h2 class="text-base font-bold mb-3">
              {t("选择安装包", "Select Package")}
            </h2>

            <Show when={scanning()}>
              <p class="text-sm text-gray-400 animate-pulse">{t("扫描中...", "Scanning...")}</p>
            </Show>

            <Show when={!scanning() && packages().length === 0}>
              <p class="text-sm text-red-400">
                {t("未找到兼容的安装包。请将 .tar.gz 文件放在程序同目录下。",
                   "No compatible packages found. Place .tar.gz files in the same directory as this app.")}
              </p>
            </Show>

            <Show when={!scanning() && packages().length > 0}>
              <div class="space-y-2 mb-4">
                <For each={packages()}>
                  {(pkg) => (
                    <label class={`flex items-start gap-3 p-3 rounded border cursor-pointer transition-colors ${
                      !pkg.compatible ? "opacity-40 cursor-not-allowed border-gray-700" :
                      selected() === pkg ? "border-indigo-500 bg-indigo-900/20" : "border-gray-700 hover:border-gray-500"
                    }`}>
                      <input type="radio" name="pkg" disabled={!pkg.compatible}
                        checked={selected() === pkg}
                        onChange={() => setSelected(pkg)}
                        class="mt-1 w-4 h-4 shrink-0" />
                      <div class="flex-1">
                        <div class="text-sm font-medium">{pkg.filename}</div>
                        <div class="text-xs text-gray-400">
                          {pkg.is_native
                            ? t("本地安装 (Native Bundle)", "Native Bundle")
                            : t("沙盒镜像 (Sandbox Image)", "Sandbox Image")}
                          {" — "}{pkg.size_mb} MB
                        </div>
                        <Show when={pkg.needs_sandbox_backend && !pkg.sandbox_backend_available}>
                          <div class="text-xs text-yellow-400 mt-1">
                            {pkg.platform === "lima"
                              ? t("⚠ 需要安装 Lima", "⚠ Lima required")
                              : t("⚠ 需要安装 WSL2（安装后需重启）", "⚠ WSL2 required (restart needed)")}
                          </div>
                        </Show>
                        <Show when={!pkg.compatible}>
                          <div class="text-xs text-red-400 mt-1">
                            {t("不兼容当前平台", "Incompatible with this platform")}
                          </div>
                        </Show>
                      </div>
                    </label>
                  )}
                </For>
              </div>

              <button class="w-full py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-sm disabled:opacity-50"
                disabled={!selected() || !selected()?.compatible}
                onClick={() => selected()?.is_native ? doInstall() : setStep(2)}>
                {selected()?.is_native
                  ? t("安装", "Install")
                  : t("下一步", "Next")}
              </button>
            </Show>
          </div>
        </Show>

        {/* Step 2: Proxy (sandbox only) */}
        <Show when={step() === 2}>
          <div class="bg-gray-800 rounded-xl p-5 border border-gray-700">
            <h2 class="text-base font-bold mb-3">
              {t("网络代理配置", "Proxy Configuration")}
            </h2>
            <div class="space-y-3 mb-4">
              <label class="flex items-center gap-2 cursor-pointer">
                <input type="radio" name="proxy" checked={proxyMode() === "system"}
                  onChange={() => setProxyMode("system")} />
                <span class="text-sm">
                  {t("使用系统代理", "Use system proxy")}
                  {systemProxy() && <span class="text-xs text-gray-400 ml-1">({systemProxy()})</span>}
                </span>
              </label>
              <label class="flex items-center gap-2 cursor-pointer">
                <input type="radio" name="proxy" checked={proxyMode() === "manual"}
                  onChange={() => setProxyMode("manual")} />
                <span class="text-sm">{t("手动输入", "Manual input")}</span>
              </label>
              <Show when={proxyMode() === "manual"}>
                <input type="text" placeholder="http://127.0.0.1:8080"
                  value={manualProxy()} onInput={e => setManualProxy(e.currentTarget.value)}
                  class="w-full bg-gray-900 border border-gray-600 rounded px-3 py-2 text-sm" />
              </Show>
              <label class="flex items-center gap-2 cursor-pointer">
                <input type="radio" name="proxy" checked={proxyMode() === "none"}
                  onChange={() => setProxyMode("none")} />
                <span class="text-sm">{t("不使用代理", "No proxy")}</span>
              </label>
            </div>
            <div class="flex gap-2">
              <button class="flex-1 py-2 bg-gray-700 hover:bg-gray-600 rounded text-sm"
                onClick={() => setStep(1)}>{t("上一步", "Back")}</button>
              <button class="flex-1 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-sm"
                onClick={doInstall}>{t("安装", "Install")}</button>
            </div>
          </div>
        </Show>

        {/* Step 3: Installing */}
        <Show when={step() === 3}>
          <div class="bg-gray-800 rounded-xl p-5 border border-gray-700">
            <h2 class="text-base font-bold mb-3">
              {installDone() ? t("安装完成", "Installation Complete")
                : installError() ? t("安装失败", "Installation Failed")
                : t("安装中...", "Installing...")}
            </h2>

            <div class="bg-gray-950 rounded p-3 h-48 overflow-y-auto font-mono text-xs mb-4">
              <For each={installLogs()}>
                {(line) => (
                  <div class={
                    line.includes("✓") ? "text-green-400"
                    : line.includes("ERROR") || line.includes("✗") ? "text-red-400"
                    : "text-gray-400"
                  }>{line}</div>
                )}
              </For>
              <Show when={installing()}>
                <div class="text-indigo-400 animate-pulse">{t("处理中...", "Working...")}</div>
              </Show>
            </div>

            <Show when={installError()}>
              <div class="p-2 bg-red-900/30 border border-red-700 rounded text-xs text-red-400 mb-3">
                {installError()}
              </div>
            </Show>

            <Show when={installDone()}>
              <button class="w-full py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-sm"
                onClick={enterMain}>
                {t("进入 ClawEnv", "Enter ClawEnv")}
              </button>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}

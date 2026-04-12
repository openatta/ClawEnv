import { createSignal, onCleanup, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { Instance } from "../../types";
type InstallProgress = { message: string; percent: number; stage: string };
type ConnTestResult = { endpoint: string; ok: boolean; message: string };
type SystemProxy = { detected: boolean; source: string; http_proxy: string; https_proxy: string; no_proxy: string };
type CheckItem = { name: string; ok: boolean; detail: string; info_only?: boolean };
type SystemCheckInfo = { os: string; arch: string; memory_gb: number; disk_free_gb: number; sandbox_backend: string; sandbox_available: boolean; checks: CheckItem[] };

function makeInstallStages(name: string) {
  return [
    { key: "detect_platform", label: "检测平台 / Detect Platform" },
    { key: "ensure_prerequisites", label: "检查前置条件 / Prerequisites" },
    { key: "create_vm", label: "创建虚拟机 / Create VM" },
    { key: "boot_vm", label: "启动虚拟机 / Boot VM" },
    { key: "configure_proxy", label: "配置代理 / Configure Proxy" },
    { key: "install_deps", label: "安装依赖 / Install Dependencies" },
    { key: "install_open_claw", label: `安装 ${name} / Install ${name}` },
    { key: "store_api_key", label: "存储 API Key / Store API Key" },
    { key: "install_browser", label: "安装浏览器 / Install Browser" },
    { key: "start_open_claw", label: `启动 ${name} / Start ${name}` },
    { key: "save_config", label: "保存配置 / Save Config" },
  ];
}

export default function InstallWizard(props: { onComplete: (instances: Instance[]) => void; onBack?: () => void; defaultInstanceName?: string; clawType?: string }) {
  const clawType = () => props.clawType || "openclaw";
  // Derive a display name — capitalize first letter of each word for now.
  // Once the claw registry IPC is available, this could be fetched from backend.
  const clawDisplayName = () => clawType().split("-").map(w => w.charAt(0).toUpperCase() + w.slice(1)).join("");
  const INSTALL_STAGES = makeInstallStages(clawDisplayName());
  const [step, setStep] = createSignal(1);
  const totalSteps = 7;

  // Step 2
  const [sysCheck, setSysCheck] = createSignal<SystemCheckInfo | null>(null);
  const [sysCheckLog, setSysCheckLog] = createSignal<string[]>([]);
  const [checking, setChecking] = createSignal(false);

  // Step 3
  const [systemProxy, setSystemProxy] = createSignal<SystemProxy | null>(null);
  const [proxyMode, setProxyMode] = createSignal<"system" | "custom" | "none">("system");
  const [httpProxy, setHttpProxy] = createSignal("");
  const [httpsProxy, setHttpsProxy] = createSignal("");
  const [connTesting, setConnTesting] = createSignal(false);
  const [connResults, setConnResults] = createSignal<ConnTestResult[]>([]);
  const [connLog, setConnLog] = createSignal<string[]>([]);

  // Instance name + validation
  const [instanceName, setInstanceName] = createSignal(props.defaultInstanceName || "default");
  const [existingNames, setExistingNames] = createSignal<string[]>([]);

  // Fetch existing instance names on mount
  (async () => {
    try {
      const list = await invoke<{ name: string }[]>("list_instances");
      setExistingNames(list.map(i => i.name));
    } catch { /* ignore */ }
  })();

  const nameError = () => {
    const name = instanceName();
    if (!name) return "Name is required";
    if (name.length > 63) return "Name too long (max 63)";
    if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/.test(name)) return "Only letters, numbers, underscore, hyphen. Must start with letter/number.";
    if (existingNames().includes(name)) return `Instance "${name}" already exists`;
    return "";
  };

  // Step 4
  const [installMethod, setInstallMethod] = createSignal<"online" | "local" | "native">("online");
  const [localFilePath, setLocalFilePath] = createSignal("");
  const [installMcpBridge, setInstallMcpBridge] = createSignal(true); // default ON
  const [installBrowser, setInstallBrowser] = createSignal(false);

  // Step 5
  const [apiKey, setApiKey] = createSignal("");
  const [apiKeyTesting, setApiKeyTesting] = createSignal(false);
  const [apiKeyResult, setApiKeyResult] = createSignal<{ ok: boolean; msg: string } | null>(null);

  // Step 6
  const [progress, setProgress] = createSignal(0);
  const [progressMessage, setProgressMessage] = createSignal("");
  const [installing, setInstalling] = createSignal(false);
  const [installError, setInstallError] = createSignal("");
  const [installLogs, setInstallLogs] = createSignal<string[]>([]);
  const [completedStages, setCompletedStages] = createSignal<Set<string>>(new Set<string>());
  const [currentStage, setCurrentStage] = createSignal("");

  let unlistenProgress: UnlistenFn | null = null;
  let unlistenComplete: UnlistenFn | null = null;
  let unlistenFailed: UnlistenFn | null = null;
  let unlistenConnStep: UnlistenFn | null = null;

  onCleanup(() => { unlistenProgress?.(); unlistenComplete?.(); unlistenFailed?.(); unlistenConnStep?.(); });

  const [iLang, setILang] = createSignal<"zh-CN" | "en">("zh-CN");
  const stepLabelsMap = {
    "zh-CN": ["欢迎", "系统检测", "网络设置", "安装方案", "API 密钥", "安装中", "完成"],
    en: ["Welcome", "System Check", "Network", "Install Plan", "API Key", "Installing", "Complete"],
  };
  const stepLabels = () => stepLabelsMap[iLang()];

  // === Step 2: System Check ===
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
    } catch (e) {
      setSysCheckLog(l => [...l, `ERROR: ${e}`]);
    } finally {
      setChecking(false);
    }
  }

  // === Step 3: Proxy & Connectivity ===
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

  async function testConnectivity() {
    setConnTesting(true);
    setConnResults([]);
    setConnLog(l => [...l, "", `--- Testing connectivity (mode: ${proxyMode()}) ---`]);

    // Listen for step-by-step events
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

  // === Step 5: API Key Test ===
  async function testApiKey() {
    setApiKeyTesting(true);
    setApiKeyResult(null);
    try {
      const msg = await invoke<string>("test_api_key", { apiKey: apiKey() });
      setApiKeyResult({ ok: true, msg });
    } catch (e) {
      setApiKeyResult({ ok: false, msg: String(e) });
    } finally {
      setApiKeyTesting(false);
    }
  }

  // === Step 6: Install ===
  async function startInstall() {
    setInstalling(true); setInstallError(""); setProgress(0);
    setProgressMessage("Starting..."); setInstallLogs([]);
    setCompletedStages(new Set<string>()); setCurrentStage("");

    const IDLE_TIMEOUT = 5 * 60 * 1000; // 5 min without any update → timeout
    let done = false;
    let timer: ReturnType<typeof setTimeout> = undefined!;
    function resetTimer() {
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (!done) { cleanup(); setInstalling(false); setInstallError("Installation stalled — no progress for 5 minutes"); }
      }, IDLE_TIMEOUT);
    }
    function cleanup() { done = true; clearTimeout(timer); unlistenProgress?.(); unlistenComplete?.(); unlistenFailed?.(); }
    resetTimer();

    unlistenProgress = await listen<InstallProgress>("install-progress", (ev) => {
      resetTimer(); // got update — reset idle timeout
      const p = ev.payload;
      setProgress(p.percent); setProgressMessage(p.message);
      setInstallLogs(l => [...l, `[${p.percent}%] ${p.message}`]);
      setCurrentStage(p.stage);
      const idx = INSTALL_STAGES.findIndex(s => s.key === p.stage);
      if (idx > 0) {
        setCompletedStages(prev => {
          const next = new Set<string>(prev);
          for (let i = 0; i < idx; i++) next.add(INSTALL_STAGES[i].key);
          return next;
        });
      }
    });

    unlistenComplete = await listen("install-complete", () => {
      clearTimeout(timer); cleanup(); setInstalling(false);
      setCompletedStages(new Set<string>(INSTALL_STAGES.map(s => s.key)));
      setInstallLogs(l => [...l, "✓ Installation complete!"]); setStep(7);
    });

    unlistenFailed = await listen<string>("install-failed", (ev) => {
      clearTimeout(timer); cleanup(); setInstalling(false);
      setInstallError(String(ev.payload));
      setInstallLogs(l => [...l, `✗ ERROR: ${ev.payload}`]);
    });

    try {
      await invoke("install_openclaw", {
        instanceName: instanceName(), clawType: clawType(), clawVersion: "latest",
        apiKey: apiKey() || null, useNative: installMethod() === "native",
        installBrowser: installBrowser(), installMcpBridge: installMcpBridge(),
        gatewayPort: 0,
      });
    } catch (e) { clearTimeout(timer); cleanup(); setInstalling(false); setInstallError(String(e)); }
  }

  async function handleComplete() {
    try { const inst = await invoke<Instance[]>("list_instances"); props.onComplete(inst); }
    catch { props.onComplete([]); }
  }

  function goToStep(s: number) {
    setStep(s);
    if (s === 2 && !sysCheck()) runSystemCheck();
    if (s === 3) detectProxy(); // re-detect every time (proxy may be turned on/off)
    if (s === 6 && !installing()) startInstall();
  }

  // Shared log output box with auto-scroll
  function LogBox(p: { logs: string[]; height?: string }) {
    let ref_el: HTMLDivElement | undefined;
    // Auto-scroll to bottom when logs change
    const scrollToBottom = () => {
      if (ref_el) ref_el.scrollTop = ref_el.scrollHeight;
    };
    return (
      <div ref={ref_el} class={`bg-gray-950 rounded border border-gray-700 p-2 overflow-y-auto font-mono text-xs text-gray-400 ${p.height || "h-40"}`}>
        <For each={p.logs}>
          {(line) => {
            // Schedule scroll after render
            setTimeout(scrollToBottom, 10);
            return <div class={
              line.includes("ERROR") || line.includes("✗") || line.includes("fail") ? "text-red-400"
              : line.includes("✓") || line.includes("OK") || line.includes("done") ? "text-green-400"
              : line.startsWith("---") ? "text-gray-600"
              : ""
            }>{line}</div>;
          }}
        </For>
        <Show when={p.logs.length === 0}><span class="text-gray-600">Waiting...</span></Show>
      </div>
    );
  }

  return (
    <div class="flex h-screen bg-gray-900 text-white">
      {/* Sidebar */}
      <div class="w-48 bg-gray-950 border-r border-gray-800 p-4 shrink-0">
        <div class="text-base font-bold mb-5">Install</div>
        <div class="space-y-2">
          {stepLabels().map((label, idx) => {
            const num = idx + 1;
            return (
              <div class={`flex items-center gap-2 text-xs ${step() === num ? "text-white font-medium" : step() > num ? "text-green-500" : "text-gray-500"}`}>
                <div class={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] border ${step() === num ? "border-indigo-500 bg-indigo-600" : step() > num ? "border-green-500 bg-green-600" : "border-gray-600"}`}>
                  {step() > num ? "✓" : num}
                </div>
                {label}
              </div>
            );
          })}
        </div>
      </div>

      {/* Content */}
      <div class="flex-1 flex flex-col p-5 overflow-hidden">
        <div class="flex-1 overflow-y-auto">

          {/* ===== Step 1: Welcome ===== */}
          {step() === 1 && (() => {
            const zh = iLang() === "zh-CN";
            return (
              <div>
                <div class="flex items-center justify-between mb-3">
                  <h2 class="text-xl font-bold">{zh ? "欢迎使用 ClawEnv" : "Welcome to ClawEnv"}</h2>
                  <div class="flex gap-1">
                    <button class={`px-2 py-0.5 text-xs rounded ${zh ? "bg-indigo-600" : "bg-gray-700"}`} onClick={() => setILang("zh-CN")}>中文</button>
                    <button class={`px-2 py-0.5 text-xs rounded ${!zh ? "bg-indigo-600" : "bg-gray-700"}`} onClick={() => setILang("en")}>EN</button>
                  </div>
                </div>
                <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 mb-4 text-sm text-gray-300 space-y-2">
                  {zh ? (<>
                    <p><strong>ClawEnv</strong> 是 <strong>{clawDisplayName()}</strong> 的跨平台沙盒安装器与管理工具。</p>
                    <p>它在您的系统上创建安全隔离的沙盒环境（Alpine Linux），让 {clawDisplayName()} 安全运行而不影响宿主系统。</p>
                    <p class="text-gray-400">安装向导将：</p>
                    <ul class="list-disc list-inside text-gray-400 space-y-1">
                      <li>检查系统是否满足要求（操作系统、内存、磁盘空间）</li>
                      <li>配置网络和代理设置</li>
                      <li>下载并在沙盒中安装 {clawDisplayName()}</li>
                      <li>安全地将 API Key 存储在系统钥匙串中</li>
                    </ul>
                    <p class="text-gray-500 text-xs mt-2">支持平台：macOS (Lima)、Windows (WSL2)、Linux (Podman)</p>
                  </>) : (<>
                    <p><strong>ClawEnv</strong> is a cross-platform sandbox installer and manager for <strong>{clawDisplayName()}</strong>.</p>
                    <p>It creates a secure, isolated sandbox environment (Alpine Linux) on your system, so {clawDisplayName()} runs safely without affecting your host OS.</p>
                    <p class="text-gray-400">This wizard will:</p>
                    <ul class="list-disc list-inside text-gray-400 space-y-1">
                      <li>Check your system meets requirements (OS, memory, disk)</li>
                      <li>Configure network & proxy settings</li>
                      <li>Download and install {clawDisplayName()} in a sandbox</li>
                      <li>Securely store your API key in system keychain</li>
                    </ul>
                    <p class="text-gray-500 text-xs mt-2">Supported: macOS (Lima), Windows (WSL2), Linux (Podman)</p>
                  </>)}
                </div>

                {/* Instance name */}
                <div class="mt-4">
                  <label class="block text-sm text-gray-400 mb-1">
                    {zh ? "实例名称" : "Instance Name"}
                  </label>
                  <input
                    type="text"
                    value={instanceName()}
                    onInput={(e) => setInstanceName(e.currentTarget.value.replace(/[^a-zA-Z0-9_-]/g, ""))}
                    placeholder="default"
                    class={`bg-gray-800 border rounded px-3 py-2 w-64 text-sm ${nameError() ? "border-red-500" : "border-gray-600"}`}
                  />
                  {nameError() ? (
                    <p class="text-xs text-red-400 mt-1">{nameError()}</p>
                  ) : (
                    <p class="text-xs text-gray-500 mt-1">
                      {zh ? "字母、数字、连字符、下划线，用于区分多个实例" : "Letters, numbers, hyphens, underscores. Used to identify this instance."}
                    </p>
                  )}
                </div>
              </div>
            );
          })()}

          {/* ===== Step 2: System Check ===== */}
          {step() === 2 && (
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
          )}

          {/* ===== Step 3: Network ===== */}
          {step() === 3 && (
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
          )}

          {/* ===== Step 4: Install Plan ===== */}
          {step() === 4 && (
            <div>
              <h2 class="text-xl font-bold mb-4">{iLang() === "zh-CN" ? "安装方案" : "Installation Plan"}</h2>

              {/* Install Mode group */}
              <fieldset class="border border-gray-600 rounded-lg p-4 mb-4">
                <legend class="px-2 text-sm font-medium text-gray-300">{iLang() === "zh-CN" ? "安装模式" : "Install Mode"}</legend>
                <div class="space-y-2">
                  <label class="flex items-center gap-3 p-2.5 rounded border border-gray-700 cursor-pointer hover:border-gray-500">
                    <input type="radio" name="im" checked={installMethod() === "online"} onChange={() => setInstallMethod("online")} class="w-4 h-4 shrink-0" />
                    <div>
                      <div class="font-medium text-sm">{iLang() === "zh-CN" ? "沙盒 - 在线构建" : "Sandbox - Online Build"}</div>
                      <div class="text-xs text-gray-400">{iLang() === "zh-CN" ? "创建虚拟机/容器并从源安装（推荐）" : "Create VM/container and install from source (recommended)"}</div>
                    </div>
                  </label>
                  <label class="flex items-center gap-3 p-2.5 rounded border border-gray-700 cursor-pointer hover:border-gray-500">
                    <input type="radio" name="im" checked={installMethod() === "local"} onChange={() => setInstallMethod("local")} class="w-4 h-4 shrink-0" />
                    <div>
                      <div class="font-medium text-sm">{iLang() === "zh-CN" ? "沙盒 - 本地镜像" : "Sandbox - Local Image"}</div>
                      <div class="text-xs text-gray-400">{iLang() === "zh-CN" ? "导入预构建的沙盒镜像文件" : "Import a pre-built sandbox image file"}</div>
                    </div>
                  </label>
                  <label class="flex items-center gap-3 p-2.5 rounded border border-dashed border-yellow-700/50 cursor-pointer hover:border-yellow-600/50">
                    <input type="radio" name="im" checked={installMethod() === "native"} onChange={() => setInstallMethod("native")} class="w-4 h-4 shrink-0" />
                    <div>
                      <div class="font-medium text-sm">{iLang() === "zh-CN" ? "本地安装" : "Local Install"}</div>
                      <div class="text-xs text-gray-400">{iLang() === "zh-CN" ? "直接安装在本机 — 无需虚拟机，启动更快" : "Install directly on this machine — no VM, faster startup"}</div>
                      <div class="text-xs text-yellow-500 mt-1">{iLang() === "zh-CN" ? "⚠ 缺少 Node.js 时会自动安装（可能需要管理员密码）" : "⚠ Node.js will be auto-installed if missing (may need admin password)"}</div>
                    </div>
                  </label>
                </div>
                <Show when={installMethod() === "local"}>
                  <input type="text" placeholder="/path/to/image.tar.gz" value={localFilePath()} onInput={e => setLocalFilePath(e.currentTarget.value)}
                    class="mt-3 bg-gray-800 border border-gray-600 rounded px-3 py-2 w-full text-sm" />
                </Show>
              </fieldset>

              {/* Optional components group */}
              <fieldset class="border border-gray-600 rounded-lg p-4">
                <legend class="px-2 text-sm font-medium text-gray-300">{iLang() === "zh-CN" ? "可选组件" : "Optional Components"}</legend>
                <div class="space-y-3">
                  {/* MCP Bridge Plugin — default ON */}
                  <label class="flex items-start gap-3 p-2.5 rounded border border-green-700/30 bg-gray-800/50 cursor-pointer hover:border-green-600/40">
                    <input type="checkbox" checked={installMcpBridge()} onChange={e => setInstallMcpBridge(e.currentTarget.checked)}
                      class="w-4 h-4 mt-0.5 shrink-0" />
                    <div>
                      <div class="text-sm font-medium">MCP Bridge Plugin <span class="text-green-400 text-xs">({iLang() === "zh-CN" ? "推荐" : "recommended"})</span></div>
                      <div class="text-xs text-gray-400 mt-1">
                        {iLang() === "zh-CN"
                          ? `使 ${clawDisplayName()} Agent 能通过安全的权限控制桥接访问宿主机的文件、命令和工具`
                          : `Enables ${clawDisplayName()} agents to access host machine files, commands, and tools through a secure, permission-controlled bridge.`}
                      </div>
                      {!installMcpBridge() && (
                        <div class="text-xs text-yellow-500 mt-1">
                          {iLang() === "zh-CN" ? "⚠ 不安装此插件，Agent 将无法访问宿主机上的程序和数据" : "⚠ Without this plugin, agents cannot access programs or data on your host machine."}
                        </div>
                      )}
                    </div>
                  </label>

                  {/* Browser Automation — default OFF */}
                  <label class="flex items-start gap-3 p-2.5 rounded border border-gray-700 bg-gray-800/50 cursor-pointer hover:border-gray-500">
                    <input type="checkbox" checked={installBrowser()} onChange={e => setInstallBrowser(e.currentTarget.checked)}
                      class="w-4 h-4 mt-0.5 shrink-0" />
                    <div>
                      <div class="text-sm font-medium">{iLang() === "zh-CN" ? "浏览器自动化（Chromium Headless）" : "Browser Automation (Chromium Headless)"}</div>
                      <div class="text-xs text-gray-400 mt-1">
                        {iLang() === "zh-CN"
                          ? "用于网页抓取、截图、CDP 自动化和验证码处理"
                          : "Required for web scraping, screenshots, CDP automation, and CAPTCHA handling."}
                      </div>
                      <div class="text-xs text-yellow-500 mt-1">
                        {iLang() === "zh-CN" ? "⚠ 增加约 630MB 空间，可稍后在设置中安装" : "⚠ Adds ~630MB. Can be installed later from Settings."}
                      </div>
                    </div>
                  </label>
                </div>
              </fieldset>
            </div>
          )}

          {/* ===== Step 5: API Key ===== */}
          {step() === 5 && (
            <div>
              <h2 class="text-xl font-bold mb-3">API Key</h2>
              <p class="text-sm text-gray-400 mb-3">Enter your {clawDisplayName()} API key. Stored securely in system keychain.</p>
              <div class="flex gap-2 items-center mb-2">
                <input type="password" placeholder="sk-..." value={apiKey()} onInput={e => setApiKey(e.currentTarget.value)}
                  class="bg-gray-800 border border-gray-600 rounded px-3 py-2 w-80 text-sm" />
                <button class="px-3 py-2 text-sm bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
                  disabled={apiKeyTesting() || !apiKey()} onClick={testApiKey}>
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
          )}

          {/* ===== Step 6: Installing ===== */}
          {step() === 6 && (
            <div class="flex flex-col h-full">
              <h2 class="text-xl font-bold mb-3">Installing...</h2>

              {/* Progress bar */}
              <div class="w-full bg-gray-800 rounded-full h-2 mb-1">
                <div class={`h-2 rounded-full transition-all ${installError() ? "bg-red-600" : "bg-indigo-600"}`}
                  style={{ width: `${progress()}%` }} />
              </div>
              <p class="text-xs text-gray-400 mb-3">{progressMessage() || "Preparing..."}</p>

              {/* Install stages checklist */}
              <div class="bg-gray-800 rounded border border-gray-700 p-2 mb-3 max-h-32 overflow-y-auto">
                <For each={INSTALL_STAGES}>
                  {(s) => {
                    const done = () => completedStages().has(s.key);
                    const active = () => currentStage() === s.key && !done();
                    const failed = () => !!(installError()) && active();
                    return (
                      <div class={`flex items-center gap-2 text-xs py-0.5 px-1 ${
                        done() ? "text-green-400" : active() ? (failed() ? "text-red-400" : "text-indigo-300") : "text-gray-600"
                      }`}>
                        <span class="w-4 text-center shrink-0">
                          {done() ? "✓" : failed() ? "✗" : active() ? "▶" : "○"}
                        </span>
                        <span>{s.label}</span>
                      </div>
                    );
                  }}
                </For>
              </div>

              {/* Log output — fills remaining space, errors show here too */}
              <div class="flex-1 min-h-0">
                <LogBox logs={installLogs()} height="h-full" />
              </div>
            </div>
          )}

          {/* ===== Step 7: Complete ===== */}
          {step() === 7 && (
            <div>
              <h2 class="text-xl font-bold mb-3">Installation Complete!</h2>
              <p class="text-sm text-gray-400 mb-3">{clawDisplayName()} is running in a secure sandbox.</p>
              <div class="bg-gray-800 rounded-lg p-3 border border-gray-700">
                <For each={INSTALL_STAGES}>
                  {(s) => <div class="flex items-center gap-2 text-xs text-green-400 py-0.5"><span>✓</span>{s.label}</div>}
                </For>
              </div>
            </div>
          )}
        </div>

        {/* Navigation */}
        <div class="flex justify-between pt-3 border-t border-gray-800 shrink-0">
          <button class="px-4 py-1.5 text-sm bg-gray-800 hover:bg-gray-700 rounded disabled:opacity-50 disabled:cursor-not-allowed"
            disabled={(step() === 1 && !props.onBack) || step() === 6 || step() === 7}
            onClick={() => {
              if (step() === 1 && props.onBack) { props.onBack(); }
              else { goToStep(step() - 1); }
            }}>
            {step() === 1 ? (iLang() === "zh-CN" ? "返回" : "Back") : (iLang() === "zh-CN" ? "上一步" : "Previous")}
          </button>
          <div class="flex gap-2">
            {/* Step 5: Skip & Install vs Install */}
            <Show when={step() === 5}>
              <button class="px-4 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                onClick={() => { setApiKey(""); goToStep(6); }}>
                Skip & Install
              </button>
              <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
                disabled={!apiKey()}
                onClick={() => goToStep(6)}>
                Install
              </button>
            </Show>
            {/* Step 6: Error — restart needed or retry */}
            <Show when={step() === 6 && installError()}>
              <Show when={installError().toLowerCase().includes("restart")}>
                <button class="px-4 py-1.5 text-sm bg-orange-600 hover:bg-orange-500 rounded"
                  onClick={async () => {
                    try { await invoke("restart_computer"); }
                    catch (e) { alert("Restart failed: " + e); }
                  }}>
                  Restart Now
                </button>
              </Show>
              <Show when={!installError().toLowerCase().includes("restart")}>
                <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
                  onClick={startInstall}>
                  Retry
                </button>
              </Show>
            </Show>
            {/* Other steps */}
            <Show when={step() < 5 && step() < totalSteps}>
              <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50 disabled:cursor-not-allowed"
                disabled={(step() === 1 && !!nameError()) || (step() === 2 && (checking() || !sysCheck()))}
                onClick={() => goToStep(step() + 1)}>
                {step() === 2 && checking() ? "Checking..." : "Next"}
              </button>
            </Show>
            <Show when={step() === 7}>
              <button class="px-4 py-1.5 text-sm bg-green-600 hover:bg-green-500 rounded" onClick={handleComplete}>
                Enter ClawEnv
              </button>
            </Show>
          </div>
        </div>
      </div>
    </div>
  );
}

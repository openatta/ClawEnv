import { createSignal, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

import type { Instance, ClawType } from "../../types";
import type { InstallState } from "./types";
import { makeInstallStages } from "./types";
import StepWelcome from "./StepWelcome";
import StepSystemCheck from "./StepSystemCheck";
import StepNetwork from "./StepNetwork";
import StepInstallPlan from "./StepInstallPlan";
import StepProgress from "./StepProgress";

export default function InstallWizard(props: {
  onComplete: (instances: Instance[]) => void;
  onBack?: () => void;
  defaultInstanceName?: string;
  clawType?: string;
  clawTypes?: ClawType[];
}) {
  // If clawType is preset (e.g., from ClawPage "+"), it's locked.
  // Otherwise, user picks from Step 0.
  const isClawTypeLocked = () => !!props.clawType;
  const [selectedClawType, setSelectedClawType] = createSignal(props.clawType || "");

  const clawType = () => selectedClawType() || "openclaw";
  const clawTypeInfo = () => allClawTypes().find(c => c.id === clawType());
  const clawDisplayName = () => clawTypeInfo()?.display_name || clawType().split("-").map(w => w.charAt(0).toUpperCase() + w.slice(1)).join("");
  const INSTALL_STAGES = () => makeInstallStages(clawDisplayName());

  // If clawType is locked, start at step 1 (Welcome). Otherwise, start at step 0 (Product Selection).
  const [step, setStep] = createSignal(isClawTypeLocked() ? 1 : 0);
  const totalSteps = 7; // 0=Product, 1=Welcome, 2=SysCheck, 3=Network, 4=Plan, 5=Progress, 6=Complete

  // Shared state lifted to orchestrator
  // Fetch available claw types if not provided
  const [fetchedClawTypes, setFetchedClawTypes] = createSignal<ClawType[]>([]);
  const allClawTypes = () => props.clawTypes?.length ? props.clawTypes : fetchedClawTypes();

  const [iLang, setILang] = createSignal<"zh-CN" | "en">("zh-CN");
  const [instanceName, setInstanceName] = createSignal(props.defaultInstanceName || "default");
  const [existingNames, setExistingNames] = createSignal<string[]>([]);
  const [installMethod, setInstallMethod] = createSignal<"online" | "local" | "native" | "native-import">("online");
  const [localFilePath, setLocalFilePath] = createSignal("");
  const [installBrowser, setInstallBrowser] = createSignal(false);
  const [installMcpBridge, setInstallMcpBridge] = createSignal(true);
  const [proxyJson, setProxyJson] = createSignal<string | null>(null);
  // Gates the Step 3 (Network) "Next" button. StepNetwork toggles this via
  // onConnectedChange — false on any selection edit, true only after a
  // successful connectivity test under the current selection.
  const [netConnected, setNetConnected] = createSignal(false);
  const [installError, setInstallError] = createSignal("");

  // Track whether system check is ready (controls Next button)
  const [sysCheckReady, setSysCheckReady] = createSignal(false);

  // Fetch existing instance names and claw types on mount
  (async () => {
    try {
      const list = await invoke<{ name: string }[]>("list_instances");
      setExistingNames(list.map(i => i.name));
    } catch { /* ignore */ }
    if (!props.clawTypes?.length) {
      try {
        const types = await invoke<ClawType[]>("list_claw_types");
        setFetchedClawTypes(types);
      } catch { /* ignore */ }
    }
  })();

  const nameError = () => {
    const name = instanceName();
    if (!name) return "Name is required";
    if (name.length > 63) return "Name too long (max 63)";
    if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/.test(name)) return "Only letters, numbers, underscore, hyphen. Must start with letter/number.";
    if (existingNames().includes(name)) return `Instance "${name}" already exists`;
    return "";
  };

  const baseLabelsMap = {
    "zh-CN": ["选择产品", "欢迎", "系统检测", "网络设置", "安装方案", "安装中", "完成"],
    en: ["Product", "Welcome", "System Check", "Network", "Install Plan", "Installing", "Complete"],
  };
  // If clawType is locked, skip the product selection step in labels
  const stepLabels = () => {
    const labels = baseLabelsMap[iLang()];
    return isClawTypeLocked() ? labels.slice(1) : labels;
  };
  // Map display step index to actual step number (for sidebar highlighting)
  const displayStepOffset = () => isClawTypeLocked() ? 1 : 0;

  // Build InstallState snapshot for StepProgress
  const buildInstallState = (): InstallState => ({
    instanceName: instanceName(),
    clawType: clawType(),
    clawDisplayName: clawDisplayName(),
    installMethod: installMethod(),
    localFilePath: localFilePath(),
    installBrowser: installBrowser(),
    installMcpBridge: installMcpBridge(),
    proxyJson: proxyJson(),
    connected: netConnected(),
  });

  // Key to force remount of StepProgress on retry. Starts at 1 so Show/keyed
  // actually renders on first entry — progressKey() must be truthy.
  const [progressKey, setProgressKey] = createSignal(1);

  // Baked-in proxy detected from imported bundle — filled after install when
  // installMethod is "local" (sandbox image import). Shown on Step 7 so the
  // user can confirm / override via ClawPage's Proxy button.
  const [bakedProxy, setBakedProxy] = createSignal("");
  async function checkBakedProxy() {
    if (installMethod() !== "local") return;
    try {
      const p = await invoke<string>("check_instance_proxy_baked_in", { name: instanceName() });
      if (p && p.trim()) setBakedProxy(p.trim());
    } catch { /* ignore — offline VM or first-boot race */ }
  }

  function goToStep(s: number) {
    // Bump progressKey BEFORE transitioning into step 5 (Progress). Otherwise
    // StepProgress mounts with the old key value, runs startInstall once, then
    // sees the key change a microtask later and runs startInstall again — the
    // second call collides with the backend INSTALL_RUNNING guard and surfaces
    // as a spurious "Installation already in progress" error in the stage UI.
    if (s === 5) {
      setInstallError("");          // clear the old failure so Retry UI shows progress, not the error
      setProgressKey(k => k + 1);   // bump key → createEffect reruns startInstall
    }
    setStep(s);
  }

  async function handleComplete() {
    try { const inst = await invoke<Instance[]>("list_instances"); props.onComplete(inst); }
    catch { props.onComplete([]); }
  }

  return (
    <div class="flex h-screen bg-gray-900 text-white">
      {/* Sidebar */}
      <div class="w-48 bg-gray-950 border-r border-gray-800 p-4 shrink-0">
        <div class="text-base font-bold mb-5">Install</div>
        <div class="space-y-2">
          {stepLabels().map((label, idx) => {
            const num = idx + displayStepOffset();
            return (
              <div class={`flex items-center gap-2 text-xs ${step() === num ? "text-white font-medium" : step() > num ? "text-green-500" : "text-gray-500"}`}>
                <div class={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] border ${step() === num ? "border-indigo-500 bg-indigo-600" : step() > num ? "border-green-500 bg-green-600" : "border-gray-600"}`}>
                  {step() > num ? "✓" : idx + 1}
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
          {/* Step 0: Product Selection (only when clawType not preset) */}
          {step() === 0 && (
            <div>
              <h2 class="text-xl font-bold mb-4">{iLang() === "zh-CN" ? "选择要安装的产品" : "Choose Product to Install"}</h2>
              <Show when={allClawTypes().length === 0}>
                <div class="flex items-center gap-2 text-sm text-gray-400 py-8">
                  <span class="animate-pulse">...</span>
                  {iLang() === "zh-CN" ? "加载产品列表..." : "Loading products..."}
                </div>
              </Show>
              <Show when={allClawTypes().length > 0}>
                <div class="grid grid-cols-2 gap-4 max-w-lg">
                  <For each={allClawTypes()}>
                    {(ct) => (
                      <button
                        class={`flex flex-col items-center gap-3 p-6 rounded-xl border-2 transition-all ${
                          selectedClawType() === ct.id
                            ? "border-indigo-500 bg-indigo-900/20"
                            : "border-gray-700 bg-gray-800/50 hover:border-gray-500"
                        }`}
                        onClick={() => setSelectedClawType(ct.id)}
                      >
                        <span class="text-3xl">{ct.logo || "📦"}</span>
                        <span class="text-sm font-medium">{ct.display_name}</span>
                        <span class="text-[10px] text-gray-500">
                          {ct.package_manager !== "npm" ? ct.pip_package : ct.npm_package}
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
              <Show when={isClawTypeLocked()}>
                <p class="text-xs text-gray-500 mt-3">{iLang() === "zh-CN" ? "产品类型已锁定" : "Product type is locked"}</p>
              </Show>
            </div>
          )}

          {step() === 1 && (
            <StepWelcome
              lang={iLang()} instanceName={instanceName()} onInstanceNameChange={setInstanceName}
              nameError={nameError()} clawDisplayName={clawDisplayName()} onLangChange={setILang}
            />
          )}
          {step() === 2 && (
            <StepSystemCheck onReady={() => setSysCheckReady(true)} />
          )}
          {step() === 3 && <StepNetwork onProxyChange={setProxyJson} onConnectedChange={setNetConnected} />}
          {step() === 4 && (
            <StepInstallPlan
              lang={iLang()} installMethod={installMethod} onMethodChange={m => setInstallMethod(m as any)}
              localFilePath={localFilePath} onFilePathChange={setLocalFilePath}
              installBrowser={installBrowser} onBrowserChange={setInstallBrowser}
              installMcpBridge={installMcpBridge} onMcpBridgeChange={setInstallMcpBridge}
              clawDisplayName={clawDisplayName()}
              supportsNative={clawTypeInfo()?.supports_native}
              supportsBrowser={clawTypeInfo()?.supports_browser}
            />
          )}
          {step() === 5 && (
            // StepProgress stays mounted across retries; incrementing
            // progressKey() re-fires its internal createEffect, which restarts
            // the install. We deliberately avoid <Show keyed> here — that
            // pattern sometimes lost the install-progress listener under
            // Tauri+Solid timing (registration raced the first emitted event).
            <StepProgress
              state={buildInstallState()} stages={INSTALL_STAGES()}
              retryTrigger={progressKey}
              onComplete={() => { void checkBakedProxy(); setStep(6); }}
              onError={(msg) => setInstallError(msg)}
            />
          )}

          {/* Step 6: Complete (inline — simple enough) */}
          {step() === 6 && (
            <div>
              <h2 class="text-xl font-bold mb-3">Installation Complete!</h2>
              <p class="text-sm text-gray-400 mb-3">{clawDisplayName()} is running in a secure sandbox.</p>
              <div class="bg-gray-800 rounded-lg p-3 border border-gray-700">
                <For each={INSTALL_STAGES()}>
                  {(s) => <div class="flex items-center gap-2 text-xs text-green-400 py-0.5"><span>✓</span>{s.label}</div>}
                </For>
              </div>
              {/* Baked-in-proxy advisory for sandbox bundle imports. The bundle
                  carries /etc/profile.d/proxy.sh from the source machine — which
                  is likely unreachable on the current host's network. Point the
                  user at ClawPage's Proxy button rather than silently
                  overwriting; they may be on the same LAN and want to keep it. */}
              <Show when={bakedProxy()}>
                <div class="mt-3 p-3 rounded-lg bg-yellow-900/30 border border-yellow-700/50 text-sm">
                  <div class="text-yellow-400 font-medium mb-1">
                    {iLang() === "zh-CN" ? "⚠ 检测到导入包自带代理" : "⚠ Imported bundle contains a baked-in proxy"}
                  </div>
                  <div class="text-xs text-gray-300 font-mono mb-2">{bakedProxy()}</div>
                  <div class="text-xs text-gray-400">
                    {iLang() === "zh-CN"
                      ? "来自导出机器的代理配置。如果当前网络不同，请在主界面的 \"代理\" 按钮中调整，否则 claw 可能无法连接网络。"
                      : "This proxy comes from the exporting machine. If you're on a different network, adjust it via the \"Proxy\" button on the main page — the claw may otherwise fail to reach the network."}
                  </div>
                </div>
              </Show>
            </div>
          )}
        </div>

        {/* Navigation */}
        <div class="flex justify-between pt-3 border-t border-gray-800 shrink-0">
          <button class="px-4 py-1.5 text-sm bg-gray-800 hover:bg-gray-700 rounded disabled:opacity-50 disabled:cursor-not-allowed"
            disabled={
              (step() === 0 && !props.onBack) ||
              (step() === 1 && isClawTypeLocked() && !props.onBack) ||
              step() === 5 || step() === 6
            }
            onClick={() => {
              if ((step() === 0 || (step() === 1 && isClawTypeLocked())) && props.onBack) { props.onBack(); }
              else { goToStep(step() - 1); }
            }}>
            {(step() === 0 || (step() === 1 && isClawTypeLocked())) ? (iLang() === "zh-CN" ? "返回" : "Back") : (iLang() === "zh-CN" ? "上一步" : "Previous")}
          </button>
          <div class="flex gap-2">
            {/* Step 4 (Install Plan) is the final step with editable choices.
                Its "Install" action launches the Progress step. No more
                dedicated API-key step — each claw collects its own key
                post-install via its ClawPage management UI. */}
            <Show when={step() === 4}>
              <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
                onClick={() => goToStep(5)}>
                {iLang() === "zh-CN" ? "安装" : "Install"}
              </button>
            </Show>
            <Show when={step() === 5 && installError()}>
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
                  onClick={() => goToStep(5)}>
                  Retry
                </button>
              </Show>
            </Show>
            <Show when={step() < 4 && step() < totalSteps}>
              <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50 disabled:cursor-not-allowed"
                disabled={
                  (step() === 0 && !selectedClawType()) ||
                  (step() === 1 && !!nameError()) ||
                  (step() === 2 && !sysCheckReady()) ||
                  (step() === 3 && !netConnected())
                }
                title={step() === 3 && !netConnected()
                  ? (iLang() === "zh-CN"
                      ? "网络不通，请先完成连通性测试"
                      : "Network unreachable — pass connectivity test first")
                  : undefined}
                onClick={() => goToStep(step() + 1)}>
                Next
              </button>
            </Show>
            <Show when={step() === 6}>
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

import { createSignal, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

import type { Instance } from "../../types";
import type { InstallState } from "./types";
import { makeInstallStages } from "./types";
import StepWelcome from "./StepWelcome";
import StepSystemCheck from "./StepSystemCheck";
import StepNetwork from "./StepNetwork";
import StepInstallPlan from "./StepInstallPlan";
import StepApiKey from "./StepApiKey";
import StepProgress from "./StepProgress";

export default function InstallWizard(props: { onComplete: (instances: Instance[]) => void; onBack?: () => void; defaultInstanceName?: string; clawType?: string }) {
  const clawType = () => props.clawType || "openclaw";
  const clawDisplayName = () => clawType().split("-").map(w => w.charAt(0).toUpperCase() + w.slice(1)).join("");
  const INSTALL_STAGES = makeInstallStages(clawDisplayName());

  const [step, setStep] = createSignal(1);
  const totalSteps = 7;

  // Shared state lifted to orchestrator
  const [iLang, setILang] = createSignal<"zh-CN" | "en">("zh-CN");
  const [instanceName, setInstanceName] = createSignal(props.defaultInstanceName || "default");
  const [existingNames, setExistingNames] = createSignal<string[]>([]);
  const [installMethod, setInstallMethod] = createSignal<"online" | "local" | "native" | "native-import">("online");
  const [localFilePath, setLocalFilePath] = createSignal("");
  const [apiKey, setApiKey] = createSignal("");
  const [installBrowser, setInstallBrowser] = createSignal(false);
  const [installMcpBridge, setInstallMcpBridge] = createSignal(true);
  const [installError, setInstallError] = createSignal("");

  // Track whether system check is ready (controls Next button)
  const [sysCheckReady, setSysCheckReady] = createSignal(false);

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

  const stepLabelsMap = {
    "zh-CN": ["欢迎", "系统检测", "网络设置", "安装方案", "API 密钥", "安装中", "完成"],
    en: ["Welcome", "System Check", "Network", "Install Plan", "API Key", "Installing", "Complete"],
  };
  const stepLabels = () => stepLabelsMap[iLang()];

  // Build InstallState snapshot for StepProgress
  const buildInstallState = (): InstallState => ({
    instanceName: instanceName(),
    clawType: clawType(),
    clawDisplayName: clawDisplayName(),
    installMethod: installMethod(),
    localFilePath: localFilePath(),
    apiKey: apiKey(),
    installBrowser: installBrowser(),
    installMcpBridge: installMcpBridge(),
  });

  // Key to force remount of StepProgress on retry
  const [progressKey, setProgressKey] = createSignal(0);

  function goToStep(s: number) {
    setStep(s);
    if (s === 6) setProgressKey(k => k + 1); // remount to restart install
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
          {step() === 1 && (
            <StepWelcome
              lang={iLang()} instanceName={instanceName()} onInstanceNameChange={setInstanceName}
              nameError={nameError()} clawDisplayName={clawDisplayName()} onLangChange={setILang}
            />
          )}
          {step() === 2 && (
            <StepSystemCheck onReady={() => setSysCheckReady(true)} />
          )}
          {step() === 3 && <StepNetwork />}
          {step() === 4 && (
            <StepInstallPlan
              lang={iLang()} installMethod={installMethod} onMethodChange={m => setInstallMethod(m as any)}
              localFilePath={localFilePath} onFilePathChange={setLocalFilePath}
              installBrowser={installBrowser} onBrowserChange={setInstallBrowser}
              installMcpBridge={installMcpBridge} onMcpBridgeChange={setInstallMcpBridge}
              clawDisplayName={clawDisplayName()}
            />
          )}
          {step() === 5 && (
            <StepApiKey apiKey={apiKey} onApiKeyChange={setApiKey} clawDisplayName={clawDisplayName()} />
          )}
          {step() === 6 && (
            <span style={{ display: "contents" }} data-key={progressKey()}>
              <StepProgress
                state={buildInstallState()} stages={INSTALL_STAGES}
                onComplete={() => setStep(7)}
                onError={(msg) => setInstallError(msg)}
              />
            </span>
          )}

          {/* Step 7: Complete (inline — simple enough) */}
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
                  onClick={() => goToStep(6)}>
                  Retry
                </button>
              </Show>
            </Show>
            <Show when={step() < 5 && step() < totalSteps}>
              <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50 disabled:cursor-not-allowed"
                disabled={(step() === 1 && !!nameError()) || (step() === 2 && !sysCheckReady())}
                onClick={() => goToStep(step() + 1)}>
                Next
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

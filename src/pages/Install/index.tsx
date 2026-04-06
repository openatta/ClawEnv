import { createSignal, onCleanup, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type Instance = {
  name: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
};

type InstallProgress = {
  message: string;
  percent: number;
  stage: string;
};

type ConnTestResult = {
  endpoint: string;
  ok: boolean;
  message: string;
};

type SystemProxy = {
  detected: boolean;
  http_proxy: string;
  https_proxy: string;
  no_proxy: string;
};

// Install stages for checklist display
const INSTALL_STAGES = [
  { key: "detect_platform", label: "Detect platform" },
  { key: "ensure_prerequisites", label: "Check prerequisites" },
  { key: "create_sandbox", label: "Create sandbox environment" },
  { key: "configure_proxy", label: "Configure proxy" },
  { key: "store_api_key", label: "Store API key" },
  { key: "install_browser", label: "Install browser" },
  { key: "start_open_claw", label: "Start OpenClaw" },
  { key: "save_config", label: "Save configuration" },
];

export default function InstallWizard(props: {
  onComplete: (instances: Instance[]) => void;
}) {
  const [step, setStep] = createSignal(1);
  const totalSteps = 7;

  // Step 2: System check
  const [checking, setChecking] = createSignal(false);
  const [checkError, setCheckError] = createSignal("");
  const [checkDone, setCheckDone] = createSignal(false);

  // Step 3: Network / Proxy
  const [systemProxy, setSystemProxy] = createSignal<SystemProxy | null>(null);
  const [proxyMode, setProxyMode] = createSignal<"none" | "system" | "custom">("none");
  const [httpProxy, setHttpProxy] = createSignal("");
  const [httpsProxy, setHttpsProxy] = createSignal("");
  const [connTesting, setConnTesting] = createSignal(false);
  const [connResults, setConnResults] = createSignal<ConnTestResult[]>([]);

  // Step 4: Install plan
  const [installMethod, setInstallMethod] = createSignal<"online" | "local">("online");
  const [localFilePath, setLocalFilePath] = createSignal("");

  // Step 5: API Key
  const [apiKey, setApiKey] = createSignal("");

  // Step 6: Progress
  const [progress, setProgress] = createSignal(0);
  const [progressMessage, setProgressMessage] = createSignal("");
  const [installing, setInstalling] = createSignal(false);
  const [installError, setInstallError] = createSignal("");
  const [installLogs, setInstallLogs] = createSignal<string[]>([]);
  const [completedStages, setCompletedStages] = createSignal<Set<string>>(new Set());
  const [currentStage, setCurrentStage] = createSignal("");

  let unlistenProgress: UnlistenFn | null = null;
  let unlistenComplete: UnlistenFn | null = null;
  let unlistenFailed: UnlistenFn | null = null;

  onCleanup(() => {
    unlistenProgress?.();
    unlistenComplete?.();
    unlistenFailed?.();
  });

  const stepLabels = [
    "Welcome", "System Check", "Network", "Install Plan",
    "API Key", "Installing", "Complete",
  ];

  // === Step 2: System Check ===
  async function runSystemCheck() {
    setChecking(true);
    setCheckError("");
    try {
      await invoke("detect_launch_state");
      setCheckDone(true);
    } catch (e) {
      setCheckError(String(e));
    } finally {
      setChecking(false);
    }
  }

  // === Step 3: Detect system proxy + test connectivity ===
  async function detectSystemProxy() {
    try {
      const sp = await invoke<SystemProxy>("detect_system_proxy");
      setSystemProxy(sp);
      if (sp.detected) {
        setProxyMode("system");
        setHttpProxy(sp.http_proxy);
        setHttpsProxy(sp.https_proxy);
      }
    } catch {
      setSystemProxy({ detected: false, http_proxy: "", https_proxy: "", no_proxy: "" });
    }
  }

  function getActiveProxy(): string | null {
    if (proxyMode() === "none") return null;
    if (proxyMode() === "system") return JSON.stringify({
      enabled: true,
      http_proxy: systemProxy()?.http_proxy || "",
      https_proxy: systemProxy()?.https_proxy || "",
      no_proxy: "localhost,127.0.0.1",
      auth_required: false, auth_user: "",
    });
    return JSON.stringify({
      enabled: true,
      http_proxy: httpProxy(),
      https_proxy: httpsProxy(),
      no_proxy: "localhost,127.0.0.1",
      auth_required: false, auth_user: "",
    });
  }

  async function testConnectivity() {
    setConnTesting(true);
    setConnResults([]);
    try {
      const proxy = proxyMode() !== "none" ? getActiveProxy() : null;
      const results = await invoke<ConnTestResult[]>("test_connectivity", {
        proxyJson: proxy,
      });
      setConnResults(results);
    } catch (e) {
      setConnResults([{ endpoint: "Test", ok: false, message: String(e) }]);
    } finally {
      setConnTesting(false);
    }
  }

  // === Step 6: Install ===
  async function startInstall() {
    setInstalling(true);
    setInstallError("");
    setProgress(0);
    setProgressMessage("Starting installation...");
    setInstallLogs([]);
    setCompletedStages(new Set<string>());
    setCurrentStage("");

    const TIMEOUT = 5 * 60 * 1000;
    let done = false;

    function cleanup() {
      done = true;
      unlistenProgress?.();
      unlistenComplete?.();
      unlistenFailed?.();
    }

    const timer = setTimeout(() => {
      if (!done) { cleanup(); setInstalling(false); setInstallError("Installation timed out (5 min)"); }
    }, TIMEOUT);

    unlistenProgress = await listen<InstallProgress>("install-progress", (event) => {
      const p = event.payload;
      setProgress(p.percent);
      setProgressMessage(p.message);
      setInstallLogs((prev) => [...prev, `[${p.percent}%] ${p.message}`]);
      setCurrentStage(p.stage);
      // Mark previous stages as complete
      const idx = INSTALL_STAGES.findIndex((s) => s.key === p.stage);
      if (idx > 0) {
        setCompletedStages((prev) => {
          const next = new Set<string>(prev);
          for (let i = 0; i < idx; i++) next.add(INSTALL_STAGES[i].key);
          return next;
        });
      }
    });

    unlistenComplete = await listen("install-complete", () => {
      clearTimeout(timer); cleanup(); setInstalling(false);
      setCompletedStages(new Set<string>(INSTALL_STAGES.map((s) => s.key)));
      setInstallLogs((prev) => [...prev, "[100%] Installation complete!"]);
      setStep(7);
    });

    unlistenFailed = await listen<string>("install-failed", (event) => {
      clearTimeout(timer); cleanup(); setInstalling(false);
      setInstallError(String(event.payload));
      setInstallLogs((prev) => [...prev, `[ERROR] ${event.payload}`]);
    });

    try {
      await invoke("install_openclaw", {
        instanceName: "default",
        clawVersion: "latest",
        apiKey: apiKey() || null,
        useNative: false,
        installBrowser: false,
        gatewayPort: 3000,
      });
    } catch (e) {
      clearTimeout(timer); cleanup(); setInstalling(false);
      setInstallError(String(e));
    }
  }

  async function handleComplete() {
    try {
      const instances = await invoke<Instance[]>("list_instances");
      props.onComplete(instances);
    } catch { props.onComplete([]); }
  }

  function goToStep(s: number) {
    setStep(s);
    if (s === 2 && !checkDone() && !checking()) runSystemCheck();
    if (s === 3 && !systemProxy()) detectSystemProxy();
    if (s === 6 && !installing()) startInstall();
  }

  return (
    <div class="flex h-screen bg-gray-900 text-white">
      {/* Sidebar */}
      <div class="w-52 bg-gray-950 border-r border-gray-800 p-5 shrink-0">
        <div class="text-lg font-bold mb-6">Install</div>
        <div class="space-y-2.5">
          {stepLabels.map((label, idx) => {
            const num = idx + 1;
            return (
              <div class={`flex items-center gap-2.5 text-sm ${
                step() === num ? "text-white font-medium"
                : step() > num ? "text-green-500" : "text-gray-500"
              }`}>
                <div class={`w-5 h-5 rounded-full flex items-center justify-center text-xs border ${
                  step() === num ? "border-indigo-500 bg-indigo-600"
                  : step() > num ? "border-green-500 bg-green-600" : "border-gray-600"
                }`}>
                  {step() > num ? "\u2713" : num}
                </div>
                {label}
              </div>
            );
          })}
        </div>
      </div>

      {/* Content */}
      <div class="flex-1 flex flex-col p-6 overflow-hidden">
        <div class="flex-1 overflow-y-auto">

          {/* ===== Step 1: Welcome ===== */}
          {step() === 1 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Welcome to ClawEnv</h2>
              <p class="text-gray-400 mb-3">
                ClawEnv will install OpenClaw in a secure, isolated sandbox on your system.
              </p>
              <p class="text-gray-400">This wizard will guide you through the setup.</p>
            </div>
          )}

          {/* ===== Step 2: System Check ===== */}
          {step() === 2 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">System Check</h2>
              <Show when={checking()}>
                <p class="text-gray-400">Detecting system environment...</p>
              </Show>
              <Show when={checkError()}>
                <p class="text-red-400 mt-2">Error: {checkError()}</p>
                <button class="mt-2 px-3 py-1 text-sm bg-gray-700 hover:bg-gray-600 rounded" onClick={runSystemCheck}>
                  Retry
                </button>
              </Show>
              <Show when={checkDone() && !checking()}>
                <div class="space-y-2">
                  <div class="flex items-center gap-2 text-sm">
                    <span class="text-green-400">&#x2714;</span>
                    <span>Platform detected</span>
                  </div>
                  <div class="flex items-center gap-2 text-sm">
                    <span class="text-green-400">&#x2714;</span>
                    <span>System ready for installation</span>
                  </div>
                </div>
              </Show>
            </div>
          )}

          {/* ===== Step 3: Network ===== */}
          {step() === 3 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Network Settings</h2>

              {/* System proxy detection */}
              <div class="bg-gray-800 rounded-lg p-3 mb-4 border border-gray-700">
                <div class="text-sm font-medium mb-2">System Proxy</div>
                <Show when={systemProxy()} fallback={<p class="text-sm text-gray-400">Detecting...</p>}>
                  {systemProxy()!.detected ? (
                    <div class="text-sm text-green-400">
                      Detected: {systemProxy()!.http_proxy || systemProxy()!.https_proxy}
                    </div>
                  ) : (
                    <div class="text-sm text-gray-500">
                      No system proxy configured (HTTP_PROXY / HTTPS_PROXY not set)
                    </div>
                  )}
                </Show>
              </div>

              {/* Proxy mode selection */}
              <div class="space-y-2 mb-4">
                <label class="flex items-center gap-2 cursor-pointer text-sm">
                  <input type="radio" name="proxy-mode" checked={proxyMode() === "none"}
                    onChange={() => setProxyMode("none")} class="w-3.5 h-3.5" />
                  <span>No proxy (direct connection)</span>
                </label>
                <Show when={systemProxy()?.detected}>
                  <label class="flex items-center gap-2 cursor-pointer text-sm">
                    <input type="radio" name="proxy-mode" checked={proxyMode() === "system"}
                      onChange={() => { setProxyMode("system"); setHttpProxy(systemProxy()!.http_proxy); setHttpsProxy(systemProxy()!.https_proxy); }}
                      class="w-3.5 h-3.5" />
                    <span>Use system proxy ({systemProxy()!.http_proxy})</span>
                  </label>
                </Show>
                <label class="flex items-center gap-2 cursor-pointer text-sm">
                  <input type="radio" name="proxy-mode" checked={proxyMode() === "custom"}
                    onChange={() => setProxyMode("custom")} class="w-3.5 h-3.5" />
                  <span>Custom proxy</span>
                </label>
              </div>

              {/* Custom proxy inputs */}
              <Show when={proxyMode() === "custom"}>
                <div class="space-y-2 mb-4">
                  <div>
                    <label class="block text-xs text-gray-400 mb-1">HTTP Proxy</label>
                    <input type="text" placeholder="http://proxy:8080" value={httpProxy()}
                      onInput={(e) => setHttpProxy(e.currentTarget.value)}
                      class="bg-gray-800 border border-gray-600 rounded px-3 py-1.5 w-80 text-sm" />
                  </div>
                  <div>
                    <label class="block text-xs text-gray-400 mb-1">HTTPS Proxy (optional, defaults to HTTP)</label>
                    <input type="text" placeholder="same as HTTP if empty" value={httpsProxy()}
                      onInput={(e) => setHttpsProxy(e.currentTarget.value)}
                      class="bg-gray-800 border border-gray-600 rounded px-3 py-1.5 w-80 text-sm" />
                  </div>
                </div>
              </Show>

              {/* Test Connectivity — always visible */}
              <button
                class="px-3 py-1.5 text-sm bg-indigo-700 hover:bg-indigo-600 rounded disabled:opacity-50 mb-3"
                disabled={connTesting()}
                onClick={testConnectivity}
              >
                {connTesting() ? "Testing..." : "Test Connectivity"}
              </button>

              {/* Test results */}
              <Show when={connResults().length > 0}>
                <div class="bg-gray-950 rounded-lg border border-gray-700 p-3 text-sm font-mono">
                  <For each={connResults()}>
                    {(r) => (
                      <div class="flex items-center gap-2 py-1">
                        <span class={r.ok ? "text-green-400" : "text-red-400"}>
                          {r.ok ? "\u2714" : "\u2718"}
                        </span>
                        <span class="w-40">{r.endpoint}</span>
                        <span class={r.ok ? "text-gray-400" : "text-red-400"}>{r.message}</span>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          )}

          {/* ===== Step 4: Install Plan ===== */}
          {step() === 4 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Installation Plan</h2>
              <p class="text-gray-400 mb-4">Choose installation method:</p>
              <div class="space-y-3">
                <label class="flex items-center gap-3 p-3 rounded border border-gray-700 cursor-pointer hover:border-gray-500">
                  <input type="radio" name="install-method" checked={installMethod() === "online"}
                    onChange={() => setInstallMethod("online")} class="w-4 h-4" />
                  <div>
                    <div class="font-medium">Online Build</div>
                    <div class="text-sm text-gray-400">Download and build from source (requires internet)</div>
                  </div>
                </label>
                <label class="flex items-center gap-3 p-3 rounded border border-gray-700 cursor-pointer hover:border-gray-500">
                  <input type="radio" name="install-method" checked={installMethod() === "local"}
                    onChange={() => setInstallMethod("local")} class="w-4 h-4" />
                  <div>
                    <div class="font-medium">Local File</div>
                    <div class="text-sm text-gray-400">Use a pre-downloaded image</div>
                  </div>
                </label>
              </div>
              <Show when={installMethod() === "local"}>
                <div class="mt-3">
                  <label class="block text-sm text-gray-400 mb-1">Image file path</label>
                  <input type="text" placeholder="/path/to/image.tar.gz" value={localFilePath()}
                    onInput={(e) => setLocalFilePath(e.currentTarget.value)}
                    class="bg-gray-800 border border-gray-600 rounded px-3 py-2 w-96" />
                </div>
              </Show>
            </div>
          )}

          {/* ===== Step 5: API Key ===== */}
          {step() === 5 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">API Key (Optional)</h2>
              <p class="text-gray-400 mb-4">
                Enter your OpenClaw API key. It will be stored securely in system keychain.
                You can skip this and configure it later in Settings.
              </p>
              <input type="password" placeholder="sk-..." value={apiKey()}
                onInput={(e) => setApiKey(e.currentTarget.value)}
                class="bg-gray-800 border border-gray-600 rounded px-3 py-2 w-96" />
              <p class="text-xs text-gray-500 mt-2">Stored in macOS Keychain / Windows Credential Manager</p>
            </div>
          )}

          {/* ===== Step 6: Installing ===== */}
          {step() === 6 && (
            <div class="flex flex-col h-full">
              <h2 class="text-2xl font-bold mb-4">Installing...</h2>

              {/* Progress bar */}
              <div class="w-full bg-gray-800 rounded-full h-2 mb-2">
                <div class="bg-indigo-600 h-2 rounded-full transition-all"
                  style={{ width: `${progress()}%` }} />
              </div>
              <p class="text-sm text-gray-400 mb-4">{progressMessage() || "Preparing..."}</p>

              {/* Install stages checklist */}
              <div class="grid grid-cols-2 gap-1 mb-4">
                <For each={INSTALL_STAGES}>
                  {(stage) => {
                    const isDone = () => completedStages().has(stage.key);
                    const isCurrent = () => currentStage() === stage.key && !isDone();
                    return (
                      <div class={`flex items-center gap-2 text-sm py-0.5 ${
                        isDone() ? "text-green-400" : isCurrent() ? "text-indigo-400" : "text-gray-600"
                      }`}>
                        <span class="w-4 text-center">
                          {isDone() ? "\u2714" : isCurrent() ? "\u25B6" : "\u25CB"}
                        </span>
                        {stage.label}
                      </div>
                    );
                  }}
                </For>
              </div>

              {/* Real-time log output */}
              <div class="flex-1 min-h-0">
                <div class="bg-gray-950 rounded-lg border border-gray-700 p-3 h-full overflow-y-auto font-mono text-xs text-gray-400">
                  <For each={installLogs()}>
                    {(line) => (
                      <div class={line.includes("ERROR") ? "text-red-400" : line.includes("100%") ? "text-green-400" : ""}>
                        {line}
                      </div>
                    )}
                  </For>
                  <Show when={installLogs().length === 0}>
                    <span class="text-gray-600">Waiting for output...</span>
                  </Show>
                </div>
              </div>

              <Show when={installError()}>
                <div class="mt-3 p-3 bg-red-900/30 border border-red-700 rounded text-sm text-red-400">
                  {installError()}
                </div>
                <button class="mt-2 px-3 py-1 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={startInstall}>
                  Retry
                </button>
              </Show>
            </div>
          )}

          {/* ===== Step 7: Complete ===== */}
          {step() === 7 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Installation Complete!</h2>
              <p class="text-gray-400 mb-4">
                OpenClaw has been installed and is running in a secure sandbox.
              </p>
              <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
                <For each={INSTALL_STAGES}>
                  {(stage) => (
                    <div class="flex items-center gap-2 text-sm text-green-400 py-0.5">
                      <span>&#x2714;</span> {stage.label}
                    </div>
                  )}
                </For>
              </div>
            </div>
          )}
        </div>

        {/* Navigation */}
        <div class="flex justify-between pt-4 border-t border-gray-800 shrink-0">
          <button
            class="px-4 py-2 text-sm bg-gray-800 hover:bg-gray-700 rounded disabled:opacity-50 disabled:cursor-not-allowed"
            disabled={step() === 1 || step() === 6 || step() === 7}
            onClick={() => goToStep(Math.max(1, step() - 1))}
          >
            Previous
          </button>

          <div class="flex gap-2">
            {/* Skip button on API Key step */}
            <Show when={step() === 5}>
              <button
                class="px-4 py-2 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                onClick={() => { setApiKey(""); goToStep(6); }}
              >
                Skip
              </button>
            </Show>

            {step() < totalSteps && step() !== 6 ? (
              <button
                class="px-4 py-2 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
                onClick={() => goToStep(step() + 1)}
              >
                {step() === 5 ? (apiKey() ? "Next" : "Skip & Install") : "Next"}
              </button>
            ) : step() === 7 ? (
              <button
                class="px-4 py-2 text-sm bg-green-600 hover:bg-green-500 rounded"
                onClick={handleComplete}
              >
                Enter ClawEnv
              </button>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}

import { createSignal, onCleanup, onMount, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type Instance = {
  name: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
};

type LaunchState =
  | { type: "first_run" }
  | { type: "not_installed" }
  | { type: "upgrade_available"; instances: Instance[] }
  | { type: "ready"; instances: Instance[] };

type InstallProgress = {
  message: string;
  percent: number;
  stage: string;
};

export default function InstallWizard(props: {
  onComplete: (instances: Instance[]) => void;
}) {
  const [step, setStep] = createSignal(1);
  const totalSteps = 7;

  // Step 2: System check
  const [launchState, setLaunchState] = createSignal<LaunchState | null>(null);
  const [checkError, setCheckError] = createSignal("");
  const [checking, setChecking] = createSignal(false);

  // Step 3: Proxy
  const [proxyEnabled, setProxyEnabled] = createSignal(false);
  const [httpProxy, setHttpProxy] = createSignal("");
  const [httpsProxy, setHttpsProxy] = createSignal("");
  const [proxyTesting, setProxyTesting] = createSignal(false);
  const [proxyTestResult, setProxyTestResult] = createSignal<string | null>(null);

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

  let unlistenProgress: UnlistenFn | null = null;
  let unlistenComplete: UnlistenFn | null = null;
  let unlistenFailed: UnlistenFn | null = null;

  onCleanup(() => {
    unlistenProgress?.();
    unlistenComplete?.();
    unlistenFailed?.();
  });

  const stepLabels = [
    "Welcome",
    "System Check",
    "Network",
    "Install Plan",
    "API Key",
    "Installing",
    "Complete",
  ];

  async function runSystemCheck() {
    setChecking(true);
    setCheckError("");
    try {
      const state = await invoke<LaunchState>("detect_launch_state");
      setLaunchState(state);
    } catch (e) {
      setCheckError(String(e));
    } finally {
      setChecking(false);
    }
  }

  async function testProxy() {
    setProxyTesting(true);
    setProxyTestResult(null);
    try {
      const proxyConfig = JSON.stringify({
        enabled: proxyEnabled(),
        http_proxy: httpProxy(),
        https_proxy: httpsProxy(),
        no_proxy: "localhost,127.0.0.1",
        auth_required: false,
        auth_user: "",
      });
      await invoke("test_proxy", { proxyJson: proxyConfig });
      setProxyTestResult("success");
    } catch (e) {
      setProxyTestResult(String(e));
    } finally {
      setProxyTesting(false);
    }
  }

  async function startInstall() {
    setInstalling(true);
    setInstallError("");
    setProgress(0);
    setProgressMessage("Starting installation...");

    // 5-minute timeout
    const INSTALL_TIMEOUT_MS = 5 * 60 * 1000;
    let installFinished = false;

    function cleanup() {
      installFinished = true;
      unlistenProgress?.();
      unlistenComplete?.();
      unlistenFailed?.();
    }

    const timeoutId = setTimeout(() => {
      if (!installFinished) {
        cleanup();
        setInstalling(false);
        setInstallError("Installation timed out");
      }
    }, INSTALL_TIMEOUT_MS);

    // Listen for progress events
    unlistenProgress = await listen<InstallProgress>("install-progress", (event) => {
      setProgress(event.payload.percent);
      setProgressMessage(event.payload.message);
    });

    unlistenComplete = await listen("install-complete", () => {
      clearTimeout(timeoutId);
      cleanup();
      setInstalling(false);
      setStep(7);
    });

    unlistenFailed = await listen<string>("install-failed", (event) => {
      clearTimeout(timeoutId);
      cleanup();
      setInstalling(false);
      setInstallError(String(event.payload));
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
      clearTimeout(timeoutId);
      cleanup();
      setInstalling(false);
      setInstallError(String(e));
    }
  }

  async function handleComplete() {
    try {
      const instances = await invoke<Instance[]>("list_instances");
      props.onComplete(instances);
    } catch {
      props.onComplete([]);
    }
  }

  // Auto-run system check when entering step 2
  function goToStep(s: number) {
    setStep(s);
    if (s === 2 && !launchState() && !checking()) {
      runSystemCheck();
    }
    if (s === 6 && !installing()) {
      startInstall();
    }
  }

  return (
    <div class="flex h-screen bg-gray-900 text-white">
      {/* Sidebar progress */}
      <div class="w-56 bg-gray-950 border-r border-gray-800 p-6 shrink-0">
        <div class="text-lg font-bold mb-8">Install</div>
        <div class="space-y-3">
          {stepLabels.map((label, idx) => {
            const num = idx + 1;
            return (
              <div
                class={`flex items-center gap-3 text-sm ${
                  step() === num
                    ? "text-white font-medium"
                    : step() > num
                    ? "text-green-500"
                    : "text-gray-500"
                }`}
              >
                <div
                  class={`w-6 h-6 rounded-full flex items-center justify-center text-xs border ${
                    step() === num
                      ? "border-indigo-500 bg-indigo-600"
                      : step() > num
                      ? "border-green-500 bg-green-600"
                      : "border-gray-600"
                  }`}
                >
                  {step() > num ? "\u2713" : num}
                </div>
                {label}
              </div>
            );
          })}
        </div>
      </div>

      {/* Content */}
      <div class="flex-1 flex flex-col p-8">
        <div class="flex-1">
          {step() === 1 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Welcome to ClawEnv</h2>
              <p class="text-gray-400 mb-4">
                ClawEnv will install OpenClaw in a secure, isolated sandbox
                environment on your system.
              </p>
              <p class="text-gray-400">
                This wizard will guide you through the setup process.
              </p>
            </div>
          )}

          {step() === 2 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">System Check</h2>
              <Show when={checking()}>
                <p class="text-gray-400">Detecting your system environment...</p>
              </Show>
              <Show when={checkError()}>
                <p class="text-red-400 mt-2">Error: {checkError()}</p>
                <button
                  class="mt-3 px-3 py-1 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={runSystemCheck}
                >
                  Retry
                </button>
              </Show>
              <Show when={launchState() && !checking()}>
                <div class="space-y-3">
                  <div class="flex items-center gap-2">
                    <span class="text-green-400">&#x2714;</span>
                    <span>
                      State: {launchState()!.type === "first_run"
                        ? "First run (no config found)"
                        : launchState()!.type === "not_installed"
                        ? "Config exists, no instances installed"
                        : launchState()!.type === "ready"
                        ? "Ready"
                        : "Upgrade available"}
                    </span>
                  </div>
                  <Show when={"instances" in launchState()!}>
                    <div class="flex items-center gap-2">
                      <span class="text-green-400">&#x2714;</span>
                      <span>
                        Existing instances:{" "}
                        {(launchState() as { instances: Instance[] }).instances.length}
                      </span>
                    </div>
                  </Show>
                  <div class="flex items-center gap-2">
                    <span class={launchState()!.type === "first_run" ? "text-yellow-400" : "text-green-400"}>
                      {launchState()!.type === "first_run" ? "\u26A0" : "\u2714"}
                    </span>
                    <span>
                      Config: {launchState()!.type === "first_run" ? "Not found (will be created)" : "Found"}
                    </span>
                  </div>
                </div>
              </Show>
            </div>
          )}

          {step() === 3 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Network Settings</h2>
              <p class="text-gray-400 mb-4">
                Configure proxy if your network requires it (optional).
              </p>

              <label class="flex items-center gap-2 mb-4 cursor-pointer">
                <input
                  type="checkbox"
                  checked={proxyEnabled()}
                  onChange={(e) => setProxyEnabled(e.currentTarget.checked)}
                  class="w-4 h-4"
                />
                <span>Enable proxy</span>
              </label>

              <Show when={proxyEnabled()}>
                <div class="space-y-3 mb-4">
                  <div>
                    <label class="block text-sm text-gray-400 mb-1">HTTP Proxy</label>
                    <input
                      type="text"
                      placeholder="http://proxy.example.com:8080"
                      value={httpProxy()}
                      onInput={(e) => setHttpProxy(e.currentTarget.value)}
                      class="bg-gray-800 border border-gray-600 rounded px-3 py-2 w-96"
                    />
                  </div>
                  <div>
                    <label class="block text-sm text-gray-400 mb-1">HTTPS Proxy</label>
                    <input
                      type="text"
                      placeholder="http://proxy.example.com:8080"
                      value={httpsProxy()}
                      onInput={(e) => setHttpsProxy(e.currentTarget.value)}
                      class="bg-gray-800 border border-gray-600 rounded px-3 py-2 w-96"
                    />
                  </div>
                </div>

                <button
                  class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded disabled:opacity-50"
                  disabled={proxyTesting() || !httpProxy()}
                  onClick={testProxy}
                >
                  {proxyTesting() ? "Testing..." : "Test Connection"}
                </button>

                <Show when={proxyTestResult() !== null}>
                  <p class={`mt-2 text-sm ${proxyTestResult() === "success" ? "text-green-400" : "text-red-400"}`}>
                    {proxyTestResult() === "success" ? "Proxy connection successful!" : `Failed: ${proxyTestResult()}`}
                  </p>
                </Show>
              </Show>
            </div>
          )}

          {step() === 4 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Installation Plan</h2>
              <p class="text-gray-400 mb-4">Choose installation method:</p>

              <div class="space-y-3">
                <label class="flex items-center gap-3 p-3 rounded border border-gray-700 cursor-pointer hover:border-gray-500">
                  <input
                    type="radio"
                    name="install-method"
                    checked={installMethod() === "online"}
                    onChange={() => setInstallMethod("online")}
                    class="w-4 h-4"
                  />
                  <div>
                    <div class="font-medium">Online Build</div>
                    <div class="text-sm text-gray-400">
                      Download and build OpenClaw from source (requires internet)
                    </div>
                  </div>
                </label>

                <label class="flex items-center gap-3 p-3 rounded border border-gray-700 cursor-pointer hover:border-gray-500">
                  <input
                    type="radio"
                    name="install-method"
                    checked={installMethod() === "local"}
                    onChange={() => setInstallMethod("local")}
                    class="w-4 h-4"
                  />
                  <div>
                    <div class="font-medium">Local File</div>
                    <div class="text-sm text-gray-400">
                      Use a pre-downloaded installation package
                    </div>
                  </div>
                </label>
              </div>

              <Show when={installMethod() === "local"}>
                <div class="mt-4">
                  <label class="block text-sm text-gray-400 mb-1">File path</label>
                  <input
                    type="text"
                    placeholder="/path/to/openclaw-package.tar.gz"
                    value={localFilePath()}
                    onInput={(e) => setLocalFilePath(e.currentTarget.value)}
                    class="bg-gray-800 border border-gray-600 rounded px-3 py-2 w-96"
                  />
                </div>
              </Show>
            </div>
          )}

          {step() === 5 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">API Key</h2>
              <p class="text-gray-400 mb-4">
                Enter your OpenClaw API key. It will be stored securely in your
                system keychain.
              </p>
              <input
                type="password"
                placeholder="sk-..."
                value={apiKey()}
                onInput={(e) => setApiKey(e.currentTarget.value)}
                class="bg-gray-800 border border-gray-600 rounded px-3 py-2 w-96"
              />
              <p class="text-sm text-gray-500 mt-2">
                You can skip this step and add it later in Settings.
              </p>
            </div>
          )}

          {step() === 6 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Installing...</h2>
              <div class="w-96 bg-gray-800 rounded-full h-2 mt-4">
                <div
                  class="bg-indigo-600 h-2 rounded-full transition-all"
                  style={{ width: `${progress()}%` }}
                />
              </div>
              <p class="text-sm text-gray-400 mt-2">
                {progressMessage() || "Preparing installation..."}
              </p>
              <Show when={installError()}>
                <p class="text-red-400 mt-4">Error: {installError()}</p>
                <button
                  class="mt-2 px-3 py-1 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={startInstall}
                >
                  Retry
                </button>
              </Show>
            </div>
          )}

          {step() === 7 && (
            <div>
              <h2 class="text-2xl font-bold mb-4">Installation Complete!</h2>
              <p class="text-gray-400">
                OpenClaw has been installed and is running in a secure sandbox.
              </p>
            </div>
          )}
        </div>

        {/* Navigation buttons */}
        <div class="flex justify-between pt-6 border-t border-gray-800">
          <button
            class="px-4 py-2 text-sm bg-gray-800 hover:bg-gray-700 rounded disabled:opacity-50 disabled:cursor-not-allowed"
            disabled={step() === 1 || step() === 6}
            onClick={() => goToStep(Math.max(1, step() - 1))}
          >
            Previous
          </button>
          {step() < totalSteps ? (
            <button
              class="px-4 py-2 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
              disabled={step() === 6 && installing()}
              onClick={() => goToStep(Math.min(totalSteps, step() + 1))}
            >
              {step() === 5 ? "Start Install" : "Next"}
            </button>
          ) : (
            <button
              class="px-4 py-2 text-sm bg-green-600 hover:bg-green-500 rounded"
              onClick={handleComplete}
            >
              Enter ClawEnv
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

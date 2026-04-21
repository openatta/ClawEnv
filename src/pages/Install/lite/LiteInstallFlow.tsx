import { createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../../../i18n";
import type { Instance } from "../../../types";
import type { InstallComponentProps } from "../../../App";

// Reuse the main wizard pieces — Lite only contributes orchestration +
// two Lite-specific steps (scan + API-key hint).
import StepWelcome from "../StepWelcome";
import StepProgress from "../StepProgress";
import { makeInstallStages, type InstallState } from "../types";

import LiteStepScan, { type PackageInfo } from "./LiteStepScan";
import LiteStepApiKeyHint from "./LiteStepApiKeyHint";

type Step = 0 | 1 | 2 | 3 | 4;
// 0: Welcome (instance name + lang)
// 1: Scan + pick bundle
// 2: Confirm plan
// 3: Install progress
// 4: API-key hint (also where bakedProxy advisory lands)
//
// Note: no Network step. Lite bundles are offline-first — native-import
// ships a self-contained node_modules and sandbox bundles ship a full
// VM image; neither path reaches out to npm/github. Forcing a
// connectivity probe on the user would just surface false negatives
// when they're already on the air-gapped network the bundle was
// prepared for.

/** Lite's offline-bundle install flow. Drop-in replacement for
 *  `src/pages/Install/InstallWizard` — same prop contract so either
 *  can be wired into `<App installComponent={...}/>`. The `clawTypes`
 *  prop is accepted for interface parity but Lite ignores it (there's
 *  no product-selection step; claw identity comes from the bundle
 *  manifest).
 */
export default function LiteInstallFlow(props: InstallComponentProps) {
  const [step, setStep] = createSignal<Step>(0);
  const [lang, setLang] = createSignal<"zh-CN" | "en">("zh-CN");

  // Instance name: honours `defaultInstanceName` (set by the backend's
  // open_install_window IPC in secondary windows) and the user can edit
  // it in Step 0. Collision detection against existing instances is done
  // the same way InstallWizard does it.
  const [instanceName, setInstanceName] = createSignal(props.defaultInstanceName || "default");
  const [existingNames, setExistingNames] = createSignal<string[]>([]);

  // User pick from scan step. Carries claw_type + claw_version +
  // claw_display_name (from the bundle manifest, authoritative) and
  // is_native so downstream steps can tailor text / install flags.
  const [pkg, setPkg] = createSignal<PackageInfo | null>(null);

  const [installError, setInstallError] = createSignal("");

  // Monotonic retry counter for StepProgress. Bumping it re-drives the
  // install effect inside that component.
  const [progressKey, setProgressKey] = createSignal(1);

  // Baked-in proxy detected after a sandbox-bundle import — surfaced
  // on the API-key hint page so the user knows to review the Proxy
  // button before expecting network traffic to work.
  const [bakedProxy, setBakedProxy] = createSignal("");

  // Fetch existing instance names on mount (for name-collision check).
  // Same pattern as InstallWizard — avoids the user creating two
  // instances with the same name, which the backend would reject with
  // a cryptic error much later in the install.
  (async () => {
    try {
      const list = await invoke<Instance[]>("list_instances");
      setExistingNames(list.map(i => i.name));
    } catch { /* ignore — empty list is safe */ }
  })();

  // If `clawType` prop is preset (user hit "+" on a specific claw tab),
  // Lite's scanner will filter to matching-type bundles only. The
  // name defaults still apply, and we skip no steps because Lite's
  // flow has no product-selection step to skip.
  const presetClawType = () => props.clawType || "";

  // Claw identity: preferred from the picked bundle's manifest. On
  // Welcome (no pick yet) fall back to preset or a generic default
  // so the header text renders.
  const clawType = () => pkg()?.claw_type || presetClawType() || "openclaw";
  const clawDisplayName = () => pkg()?.claw_display_name
    || (presetClawType()
        ? presetClawType().charAt(0).toUpperCase() + presetClawType().slice(1)
        : "OpenClaw");

  const nameError = () => {
    const name = instanceName();
    if (!name) return "Name is required";
    if (name.length > 63) return "Name too long (max 63)";
    if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/.test(name))
      return "Only letters, numbers, underscore, hyphen. Must start with letter/number.";
    if (existingNames().includes(name))
      return `Instance "${name}" already exists`;
    return "";
  };

  // Build the InstallState shape StepProgress expects. Lite's "method"
  // is always `local` (import bundle) or `native-import` depending on
  // the pick — same semantics as the main installer's StepInstallPlan
  // after the user selects "Local Image / Native Import". proxyJson is
  // null because Lite doesn't collect proxy config (offline path).
  function buildState(): InstallState {
    const p = pkg()!;
    return {
      instanceName: instanceName(),
      clawType: clawType(),
      clawDisplayName: clawDisplayName(),
      installMethod: p.is_native ? "native-import" : "local",
      localFilePath: p.path,
      installBrowser: false,
      installMcpBridge: !p.is_native,
      proxyJson: null,
      connected: true,
    };
  }

  function goToStep(s: Step) {
    // Bump progressKey BEFORE entering step 3 so StepProgress's effect
    // observes the change and (re)runs the install. Same pattern as the
    // main app's InstallWizard.
    if (s === 3) {
      setInstallError("");
      setProgressKey(k => k + 1);
    }
    setStep(s);
  }

  async function checkBakedProxy() {
    // Sandbox-bundle imports may carry /etc/profile.d/proxy.sh from
    // the exporting machine. Surface it on the hint page so the user
    // can review before expecting network traffic to flow.
    if (pkg()?.is_native) return;
    try {
      const p = await invoke<string>("check_instance_proxy_baked_in", { name: instanceName() });
      if (p && p.trim()) setBakedProxy(p.trim());
    } catch { /* ignore — VM offline, first-boot race, etc. */ }
  }

  async function onInstallDone() {
    // Advance to API-key hint. Kick off bakedProxy check in parallel
    // (doesn't block the UI — just populates the banner when resolved).
    void checkBakedProxy();
    goToStep(4);
  }

  async function onHintFinish() {
    // Fetch the fresh instance list and hand it to the caller. Shape
    // matches InstallWizard's `onComplete(instances)` contract so
    // App.tsx's two wire-up sites (first-run + install_window) work
    // unchanged.
    try {
      const list = await invoke<Instance[]>("list_instances");
      props.onComplete(list);
    } catch {
      props.onComplete([]);
    }
  }

  // Navigating "Back" from Step 0:
  // - If caller provided onBack (we're in the ?mode=install secondary
  //   window), delegate — caller closes the window.
  // - Otherwise (first-run from App.tsx), there's nothing to go back to;
  //   the button is disabled.
  const canGoBackFromStart = () => !!props.onBack;

  return (
    <div class="flex h-screen bg-gray-900 text-white">
      {/* Compact sidebar: 5 visible steps. Same visual language as the
          main wizard, narrower because Lite is one column. */}
      <div class="w-40 bg-gray-950 border-r border-gray-800 p-4 shrink-0">
        <div class="text-sm font-bold mb-4">ClawLite</div>
        <div class="space-y-2">
          {(lang() === "zh-CN"
            ? ["欢迎", "选择包", "确认", "安装中", "完成"]
            : ["Welcome", "Pick", "Confirm", "Install", "Done"]
          ).map((label, idx) => (
            <div class={`flex items-center gap-2 text-xs ${
              step() === idx ? "text-white font-medium"
                : step() > idx ? "text-green-500" : "text-gray-500"
            }`}>
              <div class={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] border ${
                step() === idx ? "border-indigo-500 bg-indigo-600"
                  : step() > idx ? "border-green-500 bg-green-600"
                  : "border-gray-600"
              }`}>
                {step() > idx ? "✓" : idx + 1}
              </div>
              {label}
            </div>
          ))}
        </div>
      </div>

      <div class="flex-1 flex flex-col p-5 overflow-hidden">
        <div class="flex-1 overflow-y-auto">
          {/* Step 0 — Welcome. Same component the main installer uses.
              Lite wires name + lang like InstallWizard does. */}
          <Show when={step() === 0}>
            <StepWelcome
              lang={lang()}
              instanceName={instanceName()}
              onInstanceNameChange={setInstanceName}
              nameError={nameError()}
              clawDisplayName={clawDisplayName()}
              onLangChange={setLang}
            />
          </Show>

          {/* Step 1 — Scan + pick (lite-only). When invoked with a
              clawType prop (user hit "+" on a specific claw tab), the
              scanner filters bundles by manifest claw_type so the user
              can't accidentally pick an OpenClaw bundle while trying
              to add a Hermes instance. */}
          <Show when={step() === 1}>
            <LiteStepScan
              filterClawType={presetClawType() || undefined}
              onPick={(p) => { setPkg(p); goToStep(2); }}
              onBack={() => goToStep(0)}
            />
          </Show>

          {/* Step 2 — Confirm plan. Small review screen so the user sees
              exactly what's about to happen before we spend minutes
              installing. Keeps surprises low on an offline-first tool. */}
          <Show when={step() === 2}>
            <div class="flex flex-col h-full">
              <h2 class="text-xl font-bold mb-3">{t("确认", "Confirm")}</h2>
              <div class="bg-gray-800 rounded-lg border border-gray-700 p-4 text-sm space-y-2">
                <div class="flex justify-between">
                  <span class="text-gray-400">{t("产品", "Product")}</span>
                  <span>
                    {clawDisplayName()}
                    {pkg()?.claw_version ? ` ${pkg()!.claw_version}` : ""}
                  </span>
                </div>
                <div class="flex justify-between">
                  <span class="text-gray-400">{t("安装包", "Bundle")}</span>
                  <span class="font-mono text-xs">{pkg()?.filename}</span>
                </div>
                <div class="flex justify-between">
                  <span class="text-gray-400">{t("安装类型", "Install Mode")}</span>
                  <span>{pkg()?.is_native
                    ? t("本地 (Native)", "Native")
                    : t("沙盒 (Sandbox)", "Sandbox")}</span>
                </div>
                <div class="flex justify-between">
                  <span class="text-gray-400">{t("实例名", "Instance Name")}</span>
                  <span class="font-mono text-xs">{instanceName()}</span>
                </div>
              </div>
              <p class="text-xs text-gray-500 mt-3">
                {t("下一步开始离线安装，无需联网。",
                   "Next step runs the offline install — no network required.")}
              </p>
              <div class="flex-1" />
              <div class="flex justify-between pt-3 border-t border-gray-800 shrink-0">
                <button class="px-4 py-1.5 text-sm bg-gray-800 hover:bg-gray-700 rounded"
                  onClick={() => goToStep(1)}>{t("上一步", "Back")}</button>
                <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
                  onClick={() => goToStep(3)}>{t("下一步", "Next")}</button>
              </div>
            </div>
          </Show>

          {/* Step 3 — Install progress (shared component). Drives the
              whole install via the `install_openclaw` IPC using buildState(). */}
          <Show when={step() === 3}>
            <StepProgress
              state={buildState()}
              stages={makeInstallStages(clawDisplayName())}
              retryTrigger={progressKey}
              onComplete={onInstallDone}
              onError={(msg) => setInstallError(msg)}
            />
          </Show>

          {/* Step 4 — API-key hint (lite-only) + baked-proxy advisory. */}
          <Show when={step() === 4}>
            <div class="flex flex-col h-full">
              <Show when={bakedProxy()}>
                <div class="mb-4 p-3 rounded-lg bg-yellow-900/30 border border-yellow-700/50 text-sm shrink-0">
                  <div class="text-yellow-400 font-medium mb-1">
                    {t("⚠ 检测到导入包自带代理",
                       "⚠ Imported bundle contains a baked-in proxy")}
                  </div>
                  <div class="text-xs text-gray-300 font-mono mb-2">{bakedProxy()}</div>
                  <div class="text-xs text-gray-400">
                    {t("来自导出机器的代理配置。如果当前网络不同，请在主界面的「代理」按钮中调整，否则 claw 可能无法连接网络。",
                       "This proxy comes from the exporting machine. If you're on a different network, adjust it via the \"Proxy\" button on the main page — the claw may otherwise fail to reach the network.")}
                  </div>
                </div>
              </Show>
              <div class="flex-1 min-h-0">
                <LiteStepApiKeyHint
                  clawType={clawType()}
                  clawDisplayName={clawDisplayName()}
                  onFinish={onHintFinish}
                />
              </div>
            </div>
          </Show>
        </div>

        {/* Step 0 / Step 1 have their own internal nav buttons. Step 2+
            nav is handled inline above. We only need a top-level nav
            bar for Step 0 because StepWelcome doesn't render its own
            Back/Next buttons. */}
        <Show when={step() === 0}>
          <div class="flex justify-between pt-3 border-t border-gray-800 shrink-0">
            <button class="px-4 py-1.5 text-sm bg-gray-800 hover:bg-gray-700 rounded disabled:opacity-50 disabled:cursor-not-allowed"
              disabled={!canGoBackFromStart()}
              onClick={() => {
                if (props.onBack) props.onBack();
              }}>
              {t("返回", "Back")}
            </button>
            <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50 disabled:cursor-not-allowed"
              disabled={!!nameError()}
              onClick={() => goToStep(1)}>
              {t("下一步", "Next")}
            </button>
          </div>
        </Show>

        {/* Retry button visible only during the install-error state on
            step 3. Mirrors the main installer. */}
        <Show when={step() === 3 && installError()}>
          <div class="flex justify-end pt-3 border-t border-gray-800 shrink-0">
            <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
              onClick={() => goToStep(3)}>
              {t("重试", "Retry")}
            </button>
          </div>
        </Show>
      </div>
    </div>
  );
}


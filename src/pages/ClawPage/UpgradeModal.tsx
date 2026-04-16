import { createSignal, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UpgradeInfo, UpgradeProgress } from "../../types";

export default function UpgradeModal(props: {
  clawDisplayName: string;
  instanceName: string;
  updateInfo: UpgradeInfo;
  onClose: () => void;
  onComplete: () => void;
}) {
  const [upgrading, setUpgrading] = createSignal(false);
  const [upgradeProgress, setUpgradeProgress] = createSignal(0);
  const [upgradeMessage, setUpgradeMessage] = createSignal("");
  const [upgradeError, setUpgradeError] = createSignal("");

  onMount(async () => {
    const unProgress = await listen<UpgradeProgress>("upgrade-progress", (ev) => {
      setUpgradeProgress(ev.payload.percent);
      setUpgradeMessage(ev.payload.message);
    });
    const unComplete = await listen<string>("upgrade-complete", () => {
      setUpgrading(false);
      props.onComplete();
    });
    const unFailed = await listen<string>("upgrade-failed", (ev) => {
      setUpgrading(false);
      setUpgradeError(String(ev.payload));
    });
    onCleanup(() => { unProgress(); unComplete(); unFailed(); });
  });

  async function startUpgrade() {
    setUpgrading(true);
    setUpgradeError("");
    setUpgradeProgress(0);
    setUpgradeMessage("Starting...");
    try {
      await invoke("upgrade_instance", { name: props.instanceName, targetVersion: null });
    } catch (e) {
      setUpgrading(false);
      setUpgradeError(String(e));
    }
  }

  const dn = () => props.clawDisplayName;
  const info = () => props.updateInfo;
  const isBeta = () => /[-](beta|alpha|rc|pre|dev)/i.test(info().latest || "");

  return (
    <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-[420px] shadow-2xl">
        <h3 class="text-base font-bold mb-3">
          {upgrading() ? "Upgrading..." : `Upgrade ${dn()}`}
        </h3>
        <Show when={!upgrading()}>
          <div class="text-sm text-gray-300 mb-2">
            <span class="text-gray-400">Current:</span> {info().current}
          </div>
          <div class="text-sm text-gray-300 mb-4">
            <span class="text-gray-400">Latest:</span> <span class="text-green-400">{info().latest}</span>
            {info().security && <span class="text-red-400 ml-2">Security</span>}
            {isBeta() && <span class="text-yellow-400 ml-2">(beta)</span>}
          </div>
          <p class="text-xs text-gray-500 mb-4">
            The upgrade will stop the gateway, update {dn()}, and restart.
          </p>
          <div class="flex gap-2 justify-end">
            <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
              onClick={props.onClose}>Cancel</button>
            <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
              onClick={startUpgrade}>Upgrade Now</button>
          </div>
        </Show>
        <Show when={upgrading()}>
          <div class="w-full bg-gray-700 rounded-full h-2 mb-2">
            <div class="h-2 rounded-full bg-indigo-600 transition-all" style={{ width: `${upgradeProgress()}%` }} />
          </div>
          <p class="text-xs text-gray-400 mb-2">{upgradeMessage()}</p>
          {upgradeError() && (
            <div>
              <p class="text-xs text-red-400 mb-3">{upgradeError()}</p>
              <div class="flex gap-2 justify-end">
                <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
                  onClick={() => { props.onClose(); }}>Close</button>
                <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded"
                  onClick={startUpgrade}>Retry</button>
              </div>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}

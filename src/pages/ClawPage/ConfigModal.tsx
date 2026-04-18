import { createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { Instance, ClawType } from "../../types";
import { t } from "../../i18n";

export default function ConfigModal(props: {
  instanceName: string;
  instances: Instance[];
  clawType: ClawType;
  sandboxType: string;
  gatewayPort: number;
  ttydPort: number;
  onSave: () => void;
  onClose: () => void;
}) {
  const [cfgGatewayPort, setCfgGatewayPort] = createSignal(props.gatewayPort);
  const [cfgTtydPort, setCfgTtydPort] = createSignal(props.ttydPort);
  const [cfgSaving, setCfgSaving] = createSignal(false);
  const [cfgError, setCfgError] = createSignal("");
  const [caps, setCaps] = createSignal<Record<string, boolean>>({});
  let gatewayPortRef: HTMLInputElement | undefined;
  let ttydPortRef: HTMLInputElement | undefined;

  const isNative = () => props.sandboxType?.toLowerCase() === "native";

  // Load capabilities on mount
  (async () => {
    try {
      const c = await invoke<Record<string, boolean>>("get_instance_capabilities", { name: props.instanceName });
      setCaps(c);
    } catch { setCaps({}); }
    // Set initial values after render
    setTimeout(() => {
      if (gatewayPortRef) gatewayPortRef.value = String(props.gatewayPort);
      if (ttydPortRef) ttydPortRef.value = String(props.ttydPort);
    }, 0);
  })();

  const portConflict = () => {
    const gp = cfgGatewayPort();
    const tp = cfgTtydPort();
    if (gp === tp) return "Gateway and terminal ports cannot be the same";
    // Check against every port a sibling instance holds: gateway, ttyd,
    // AND dashboard (Hermes). The dashboard_port check was missing in
    // v0.2.6 — a user could pick a gateway_port that landed on another
    // instance's Hermes dashboard port (e.g. 3005 when instance A has
    // dashboard at 3005) and the UI would silently accept it. Guard by
    // coercing `undefined` (older instance records) to 0 so they never
    // match a real port.
    for (const inst of props.instances) {
      if (inst.name === props.instanceName) continue;
      const used = [inst.gateway_port, inst.ttyd_port, inst.dashboard_port ?? 0]
        .filter(p => p !== 0);
      if (used.includes(gp)) return `Port ${gp} already used by "${inst.name}"`;
      if (used.includes(tp)) return `Port ${tp} already used by "${inst.name}"`;
    }
    if (gp < 1024 || gp > 65535) return "Gateway port must be 1024-65535";
    if (tp < 1024 || tp > 65535) return "Terminal port must be 1024-65535";
    return "";
  };

  async function saveConfig(restart: boolean) {
    const conflict = portConflict();
    if (conflict) { setCfgError(conflict); return; }
    setCfgSaving(true); setCfgError("");
    try {
      await invoke("edit_instance_ports", {
        name: props.instanceName,
        gatewayPort: cfgGatewayPort(),
        ttydPort: cfgTtydPort(),
      });
      if (restart) {
        await invoke("start_instance", { name: props.instanceName });
      }
      props.onSave();
    } catch (e) { setCfgError(String(e)); }
    finally { setCfgSaving(false); }
  }

  return (
    <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-96 shadow-2xl">
        <h3 class="text-base font-bold mb-4">Configure — {props.instanceName}</h3>
        <div class="space-y-3">
          <div>
            <label class="block text-xs text-gray-400 mb-1">Gateway Port</label>
            <input ref={gatewayPortRef} type="number"
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value) || props.clawType.default_port;
                setCfgGatewayPort(v);
                setCfgTtydPort(v + 4681);
                if (ttydPortRef) ttydPortRef.value = String(v + 4681);
              }}
              class="bg-gray-900 border border-gray-600 rounded px-3 py-1.5 w-full text-sm" />
          </div>
          <div>
            <label class="block text-xs text-gray-400 mb-1">Terminal (ttyd) Port</label>
            <input ref={ttydPortRef} type="number"
              disabled={isNative()}
              onInput={(e) => setCfgTtydPort(parseInt(e.currentTarget.value) || 7681)}
              class={`bg-gray-900 border border-gray-600 rounded px-3 py-1.5 w-full text-sm ${isNative() ? "opacity-40 cursor-not-allowed" : ""}`} />
            <Show when={isNative()}>
              <span class="text-[10px] text-gray-500">N/A for native mode</span>
            </Show>
          </div>
        </div>
        <Show when={!caps().port_edit}>
          <p class="text-xs text-gray-500 mt-2">Port forwarding not supported by this backend.</p>
        </Show>
        {portConflict() && <p class="text-xs text-red-400 mt-2">{portConflict()}</p>}
        {cfgError() && !portConflict() && <p class="text-xs text-red-400 mt-2">{cfgError()}</p>}
        <p class="text-xs text-yellow-500 mt-2">Port changes require instance restart.</p>
        <div class="flex gap-2 justify-end mt-4">
          <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded"
            onClick={props.onClose}>Cancel</button>
          <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
            disabled={cfgSaving() || !!portConflict()} onClick={() => saveConfig(false)}>
            {cfgSaving() ? "..." : "Save"}
          </button>
          <button class="px-3 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
            disabled={cfgSaving() || !!portConflict()} onClick={() => saveConfig(true)}>
            {cfgSaving() ? "..." : "Save & Restart"}
          </button>
        </div>
      </div>
    </div>
  );
}

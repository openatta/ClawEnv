import { createSignal, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import IconBar from "../components/IconBar";
import Home from "../pages/Home";
import ClawPage from "../pages/ClawPage/index";
import SandboxPage from "../pages/SandboxPage";
import Settings from "../pages/Settings";
import { AppContext } from "../context";
import type { Instance, ClawType } from "../types";

export type Page = "home" | "sandbox" | "settings" | `claw:${string}`;

/**
 * Canonical payload of the backend's `instance-changed` event. Every IPC that
 * mutates an instance's runtime or config state emits this, and this component
 * is the single subscriber that turns the event into UI state updates.
 *
 * Keep this shape in sync with tauri/src/ipc/emit.rs::InstanceChanged.
 */
export type InstanceChanged = {
  // Restart is composed of a stop+start pair on the backend (two separate
  // IPCs, each emitting its own instance-changed) rather than a single
  // atomic "restart" action, so the union has no "restart" variant.
  action:
    | "install" | "start" | "stop" | "delete" | "rename"
    | "edit_ports" | "edit_resources" | "install_chromium" | "upgrade";
  instance?: string | null;
  old_name?: string;
  new_name?: string;
  removed?: boolean;
  needs_restart?: boolean;
};


export default function MainLayout(props: { instances: Instance[] }) {
  const [activePage, setActivePage] = createSignal<Page>("home");
  const [healths, setHealths] = createSignal<Record<string, string>>({});
  const [instances, setInstances] = createSignal<Instance[]>(props.instances);
  const [clawTypes, setClawTypes] = createSignal<ClawType[]>([]);
  // Monotonic counter bumped whenever a start/restart makes a cached gateway
  // token stale. Children (ClawPage) watch this via the context and refetch.
  const [tokenEpoch, setTokenEpoch] = createSignal(0);
  // Transient banner surfaced when the backend signals needs_restart after an
  // edit_ports / edit_resources action. Null when no banner is active.
  const [restartHint, setRestartHint] = createSignal<string | null>(null);

  async function refreshInstances() {
    try {
      const list = await invoke<Instance[]>("list_instances");
      setInstances(list);
    } catch { /* keep current */ }
    refreshHealths();
  }

  async function refreshOneHealth(name: string) {
    try {
      const h = await invoke<string>("get_instance_health", { name });
      setHealths((prev) => ({ ...prev, [name]: h }));
    } catch {
      setHealths((prev) => ({ ...prev, [name]: "unreachable" }));
    }
  }

  async function refreshHealths() {
    for (const inst of instances()) {
      await refreshOneHealth(inst.name);
    }
  }

  onMount(async () => {
    try {
      const types = await invoke<ClawType[]>("list_claw_types");
      setClawTypes(types);
    } catch (e) {
      console.error("Failed to load claw types:", e);
    }

    refreshHealths();

    const unHealth = await listen<{ instance_name: string; health: string }>(
      "instance-health",
      (event) => {
        setHealths((prev) => ({
          ...prev,
          [event.payload.instance_name]: event.payload.health,
        }));
      }
    );

    // NOTE: legacy `instances-changed` (plural) event was retired — all
    // state-changing IPCs now emit the richer `instance-changed` (singular)
    // event handled below, carrying action / needs_restart / rename / removed
    // semantics. Keeping both would double-fire the refresh pipeline.
    //
    // Unified state-sync entry point. Every mutating IPC on the backend emits
    // `instance-changed` after its side effect lands. This is the ONE place
    // that decides what to refresh — scattered post-action `onInstancesChanged`
    // calls across child components are redundant and race-prone.
    const unInstChanged = await listen<InstanceChanged>("instance-changed", async (ev) => {
      const p = ev.payload;
      const target = p.instance ?? undefined;

      // Refresh instance list always (delete/rename/install_chromium may change shape)
      await refreshInstances();

      // Refresh health for the affected instance specifically — instance-health
      // polling runs every 5-10s, but an explicit action deserves a fast read.
      if (target) await refreshOneHealth(target);

      // Start invalidates any cached gateway token (VM process restarted,
      // token file regenerated). Bump the epoch so ClawPage refetches.
      // doAction("restart") on the frontend fires stop+start back-to-back,
      // so the trailing "start" event handles the restart case too.
      if (p.action === "start") {
        setTokenEpoch((n) => n + 1);
      }

      // Rename + active tab points at old name → auto-switch to new tab.
      if (p.action === "rename" && p.old_name && p.new_name) {
        const page = activePage();
        if (page === `claw:${p.old_name}`) {
          setActivePage(`claw:${p.new_name}` as Page);
        }
      }

      // Delete + active tab pointed at the deleted instance → fall back to home.
      if (p.removed && target) {
        const page = activePage();
        if (page === `claw:${target}`) {
          setActivePage("home");
        }
      }

      // Config-only edits (ports/resources) don't take effect until restart —
      // surface a banner so the user doesn't wonder why nothing changed.
      if (p.needs_restart && target) {
        setRestartHint(
          `配置已保存。重启实例 '${target}' 后生效 / ` +
          `Config saved. Restart '${target}' to apply.`
        );
        setTimeout(() => setRestartHint(null), 8000);
      }
    });

    onCleanup(() => { unHealth(); unInstChanged(); });
  });

  const interval = setInterval(refreshHealths, 10000);
  onCleanup(() => clearInterval(interval));

  async function openInstallWindow(clawType?: string) {
    try {
      await invoke("open_install_window", { instanceName: null, clawType: clawType || null });
    } catch (e) {
      console.error("Failed to open install window:", e);
    }
  }

  // Derive the active claw type ID from the page
  const activeClawType = () => {
    const page = activePage();
    if (page.startsWith("claw:")) return page.slice(5);
    return null;
  };

  const activeClawInstances = () => {
    const ct = activeClawType();
    if (!ct) return [];
    return instances().filter((i) => i.claw_type === ct);
  };

  const activeClawDesc = () => {
    const ct = activeClawType();
    if (!ct) return null;
    return clawTypes().find((t) => t.id === ct) || null;
  };

  return (
    <AppContext.Provider value={{
      instances, healths, clawTypes, refreshInstances, openInstallWindow,
      tokenEpoch,
    }}>
      <div class="flex h-screen bg-gray-900 text-white relative">
        <IconBar
          activePage={activePage()}
          onNavigate={setActivePage}
          clawTypes={clawTypes()}
          instances={instances()}
        />
        <main class="flex-1 overflow-hidden">
          {activePage() === "home" && (
            <Home
              instances={instances()}
              healths={healths()}
              onHealthChange={refreshInstances}
              clawTypes={clawTypes()}
              onAddInstance={openInstallWindow}
            />
          )}
          {activeClawType() && activeClawDesc() && (
            <ClawPage
              clawType={activeClawDesc()!}
              instances={activeClawInstances()}
              healths={healths()}
              onInstancesChanged={refreshInstances}
              onAddInstance={() => openInstallWindow(activeClawType()!)}
            />
          )}
          {activePage() === "sandbox" && <SandboxPage />}
          {activePage() === "settings" && <Settings />}
        </main>
        {restartHint() && (
          <div class="absolute bottom-4 right-4 max-w-md bg-amber-900 border border-amber-600 text-amber-100 text-xs rounded px-3 py-2 shadow-lg z-50">
            {restartHint()}
          </div>
        )}
      </div>
    </AppContext.Provider>
  );
}

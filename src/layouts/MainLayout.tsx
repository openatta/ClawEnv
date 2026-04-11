import { createSignal, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import IconBar from "../components/IconBar";
import Home from "../pages/Home";
import ClawPage from "../pages/ClawPage";
import SandboxPage from "../pages/SandboxPage";
import Settings from "../pages/Settings";
import type { Instance, ClawType } from "../types";

export type Page = "home" | "sandbox" | "settings" | `claw:${string}`;


export default function MainLayout(props: { instances: Instance[] }) {
  const [activePage, setActivePage] = createSignal<Page>("home");
  const [healths, setHealths] = createSignal<Record<string, string>>({});
  const [instances, setInstances] = createSignal<Instance[]>(props.instances);
  const [clawTypes, setClawTypes] = createSignal<ClawType[]>([]);

  async function refreshInstances() {
    try {
      const list = await invoke<Instance[]>("list_instances");
      setInstances(list);
    } catch { /* keep current */ }
    refreshHealths();
  }

  async function refreshHealths() {
    for (const inst of instances()) {
      try {
        const h = await invoke<string>("get_instance_health", { name: inst.name });
        setHealths((prev) => ({ ...prev, [inst.name]: h }));
      } catch {
        setHealths((prev) => ({ ...prev, [inst.name]: "unreachable" }));
      }
    }
  }

  onMount(async () => {
    // Load claw type registry
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

    const unChanged = await listen("instances-changed", () => {
      refreshInstances();
    });

    onCleanup(() => { unHealth(); unChanged(); });
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

  // Filter instances for the active claw type
  const activeClawInstances = () => {
    const ct = activeClawType();
    if (!ct) return [];
    return instances().filter((i) => i.claw_type === ct);
  };

  // Get the ClawType descriptor for the active page
  const activeClawDesc = () => {
    const ct = activeClawType();
    if (!ct) return null;
    return clawTypes().find((t) => t.id === ct) || null;
  };

  return (
    <div class="flex h-screen bg-gray-900 text-white">
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
    </div>
  );
}

import { createSignal, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import IconBar from "../components/IconBar";
import Home from "../pages/Home";
import OpenClawPage from "../pages/OpenClawPage";
import SandboxPage from "../pages/SandboxPage";
import Settings from "../pages/Settings";

type Instance = {
  name: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
  ttyd_port: number;
};

type Page = "home" | "openclaw" | "sandbox" | "settings";

export default function MainLayout(props: { instances: Instance[] }) {
  const [activePage, setActivePage] = createSignal<Page>("home");
  const [healths, setHealths] = createSignal<Record<string, string>>({});
  const [instances, setInstances] = createSignal<Instance[]>(props.instances);

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
    refreshHealths();

    // Listen for health events from monitor
    const unHealth = await listen<{ instance_name: string; health: string }>(
      "instance-health",
      (event) => {
        setHealths((prev) => ({
          ...prev,
          [event.payload.instance_name]: event.payload.health,
        }));
      }
    );

    // Listen for instance changes (e.g. install window completed)
    const unChanged = await listen("instances-changed", () => {
      refreshInstances();
    });

    onCleanup(() => { unHealth(); unChanged(); });
  });

  const interval = setInterval(refreshHealths, 10000);
  onCleanup(() => clearInterval(interval));

  async function openInstallWindow() {
    try {
      await invoke("open_install_window", { instanceName: null });
    } catch (e) {
      console.error("Failed to open install window:", e);
    }
  }

  return (
    <div class="flex h-screen bg-gray-900 text-white">
      <IconBar activePage={activePage()} onNavigate={setActivePage} />
      <main class="flex-1 overflow-hidden">
        {activePage() === "home" && (
          <Home instances={instances()} healths={healths()} onHealthChange={refreshInstances} />
        )}
        {activePage() === "openclaw" && (
          <OpenClawPage
            instances={instances()}
            healths={healths()}
            onInstancesChanged={refreshInstances}
            onAddInstance={openInstallWindow}
          />
        )}
        {activePage() === "sandbox" && <SandboxPage />}
        {activePage() === "settings" && <Settings />}
      </main>
    </div>
  );
}

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
};

type Page = "home" | "openclaw" | "sandbox" | "settings";

export default function MainLayout(props: { instances: Instance[] }) {
  const [activePage, setActivePage] = createSignal<Page>("home");
  const [healths, setHealths] = createSignal<Record<string, string>>({});

  // Shared health polling — one source of truth for all pages
  async function refreshHealths() {
    for (const inst of props.instances) {
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
    // Listen for monitor health events
    const unlisten = await listen<{ instance_name: string; health: string }>(
      "instance-health",
      (event) => {
        setHealths((prev) => ({
          ...prev,
          [event.payload.instance_name]: event.payload.health,
        }));
      }
    );
    onCleanup(unlisten);
  });

  // Periodic refresh
  const interval = setInterval(refreshHealths, 10000);
  onCleanup(() => clearInterval(interval));

  return (
    <div class="flex h-screen bg-gray-900 text-white">
      <IconBar activePage={activePage()} onNavigate={setActivePage} />
      <main class="flex-1 overflow-hidden">
        {activePage() === "home" && (
          <Home instances={props.instances} healths={healths()} onHealthChange={refreshHealths} />
        )}
        {activePage() === "openclaw" && (
          <OpenClawPage instances={props.instances} healths={healths()} />
        )}
        {activePage() === "sandbox" && <SandboxPage />}
        {activePage() === "settings" && <Settings />}
      </main>
    </div>
  );
}

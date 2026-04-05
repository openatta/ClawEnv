import { createSignal, For } from "solid-js";
import IconBar from "../components/IconBar";
import Home from "../pages/Home";
import OpenClawPage from "../pages/OpenClawPage";
import Settings from "../pages/Settings";

type Instance = {
  name: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
};

type Page = "home" | "openclaw" | "settings";

export default function MainLayout(props: { instances: Instance[] }) {
  const [activePage, setActivePage] = createSignal<Page>("home");

  return (
    <div class="flex h-screen bg-gray-900 text-white">
      {/* Left icon bar — 56px */}
      <IconBar activePage={activePage()} onNavigate={setActivePage} />

      {/* Content area */}
      <main class="flex-1 overflow-hidden">
        {activePage() === "home" && <Home instances={props.instances} />}
        {activePage() === "openclaw" && (
          <OpenClawPage instances={props.instances} />
        )}
        {activePage() === "settings" && <Settings />}
      </main>
    </div>
  );
}

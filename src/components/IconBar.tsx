import { JSX } from "solid-js";

type Page = "home" | "openclaw" | "settings";

// SVG icon components (20x20, stroke-based)
const icons: Record<Page | "user" | "logo", () => JSX.Element> = {
  logo: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="w-5 h-5">
      <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" fill="currentColor" stroke="none" />
    </svg>
  ),
  home: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      <polyline points="9 22 9 12 15 12 15 22" />
    </svg>
  ),
  openclaw: () => (
    <svg viewBox="0 0 24 24" class="w-5 h-5">
      {/* OpenClaw red claw icon */}
      <path d="M12 2C8 2 5 4.5 5 8c0 2 1 3.5 2.5 4.5L6 17c-.5 1.5.5 3 2 3h8c1.5 0 2.5-1.5 2-3l-1.5-4.5C18 11.5 19 10 19 8c0-3.5-3-6-7-6z" fill="#ef4444" />
      <circle cx="9" cy="7" r="1.2" fill="white" />
      <circle cx="15" cy="7" r="1.2" fill="white" />
      <path d="M8.5 12c1 1 3.5 1.5 7 0" stroke="white" stroke-width="1" fill="none" stroke-linecap="round" />
    </svg>
  ),
  settings: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  ),
  user: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
      <circle cx="12" cy="7" r="4" />
    </svg>
  ),
};

const navItems: { id: Page; label: string; position: "top" | "bottom" }[] = [
  { id: "home", label: "Home", position: "top" },
  { id: "openclaw", label: "OpenClaw", position: "top" },
  { id: "settings", label: "Settings", position: "bottom" },
];

export default function IconBar(props: {
  activePage: Page;
  onNavigate: (page: Page) => void;
}) {
  const topItems = navItems.filter((i) => i.position === "top");
  const bottomItems = navItems.filter((i) => i.position === "bottom");

  function NavBtn(p: { id: Page; label: string }) {
    const active = () => props.activePage === p.id;
    return (
      <button
        class={`relative w-10 h-10 rounded-lg flex items-center justify-center transition-colors ${
          active()
            ? "bg-gray-700 text-white"
            : "text-gray-400 hover:text-white hover:bg-gray-800"
        }`}
        onClick={() => props.onNavigate(p.id)}
        title={p.label}
      >
        {active() && (
          <div class="absolute left-0 top-2 bottom-2 w-0.5 bg-white rounded-r" />
        )}
        {icons[p.id]()}
      </button>
    );
  }

  return (
    <nav class="flex flex-col w-14 bg-gray-950 border-r border-gray-800 items-center py-3 shrink-0">
      {/* Logo */}
      <button
        class="w-9 h-9 rounded-lg bg-indigo-600 flex items-center justify-center mb-4 hover:bg-indigo-500 transition-colors text-white"
        onClick={() => props.onNavigate("home")}
        title="ClawEnv"
      >
        {icons.logo()}
      </button>

      {/* Top nav */}
      <div class="flex flex-col items-center gap-1 flex-1">
        {topItems.map((item) => <NavBtn id={item.id} label={item.label} />)}
      </div>

      {/* Bottom nav */}
      <div class="flex flex-col items-center gap-1">
        {bottomItems.map((item) => <NavBtn id={item.id} label={item.label} />)}
        {/* User — greyed out placeholder */}
        <button
          class="w-10 h-10 rounded-lg flex items-center justify-center text-gray-600 cursor-not-allowed opacity-40"
          title="User login (coming soon)"
          disabled
        >
          {icons.user()}
        </button>
      </div>
    </nav>
  );
}

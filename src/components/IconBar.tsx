import { JSX } from "solid-js";

type Page = "home" | "openclaw" | "sandbox" | "settings";

const icons: Record<Page | "user" | "logo", () => JSX.Element> = {
  logo: () => (
    <svg viewBox="0 0 120 120" fill="none" class="w-full h-full">
      <defs><linearGradient id="lg" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="#60A5FA"/><stop offset="100%" stop-color="#2563EB"/></linearGradient></defs>
      <path d="M60 10 C30 10 15 35 15 55 C15 75 30 95 45 100 L45 110 L55 110 L55 100 C55 100 60 102 65 100 L65 110 L75 110 L75 100 C90 95 105 75 105 55 C105 35 90 10 60 10Z" fill="url(#lg)"/>
      <path d="M20 45 C5 40 0 50 5 60 C10 70 20 65 25 55 C28 48 25 45 20 45Z" fill="url(#lg)"/>
      <path d="M100 45 C115 40 120 50 115 60 C110 70 100 65 95 55 C92 48 95 45 100 45Z" fill="url(#lg)"/>
      <path d="M45 15 Q35 5 30 8" stroke="#60A5FA" stroke-width="3" stroke-linecap="round"/>
      <path d="M75 15 Q85 5 90 8" stroke="#60A5FA" stroke-width="3" stroke-linecap="round"/>
      <circle cx="45" cy="35" r="6" fill="#050810"/><circle cx="75" cy="35" r="6" fill="#050810"/>
      <circle cx="46" cy="34" r="2.5" fill="#00e5cc"/><circle cx="76" cy="34" r="2.5" fill="#00e5cc"/>
    </svg>
  ),
  home: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  ),
  openclaw: () => (
    <span class="text-lg leading-none" style="font-size:20px">🦞</span>
  ),
  sandbox: () => (
    // VM/container icon — server with layers
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <rect x="2" y="2" width="20" height="8" rx="2" />
      <rect x="2" y="14" width="20" height="8" rx="2" />
      <circle cx="6" cy="6" r="1" fill="currentColor" />
      <circle cx="6" cy="18" r="1" fill="currentColor" />
      <line x1="10" y1="6" x2="18" y2="6" />
      <line x1="10" y1="18" x2="18" y2="18" />
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

const topItems: { id: Page; label: string }[] = [
  { id: "home", label: "Home" },
  { id: "openclaw", label: "OpenClaw" },
];

const bottomItems: { id: Page; label: string }[] = [
  { id: "sandbox", label: "Sandbox" },
  { id: "settings", label: "Settings" },
];

export default function IconBar(props: {
  activePage: Page;
  onNavigate: (page: Page) => void;
}) {
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
        class="w-10 h-10 rounded-lg flex items-center justify-center hover:bg-gray-800 transition-colors"
        onClick={() => props.onNavigate("home")}
        title="ClawEnv"
      >
        <span class="w-7 h-7 block">{icons.logo()}</span>
      </button>

      {/* Separator */}
      <div class="w-8 border-t border-gray-700 my-2" />

      {/* Top nav */}
      <div class="flex flex-col items-center gap-1 flex-1">
        {topItems.map((item) => <NavBtn id={item.id} label={item.label} />)}
      </div>

      {/* Bottom nav: Sandbox + Settings + User */}
      <div class="flex flex-col items-center gap-1">
        {bottomItems.map((item) => <NavBtn id={item.id} label={item.label} />)}
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

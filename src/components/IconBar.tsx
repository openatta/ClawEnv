import { JSX, createSignal, For, Show } from "solid-js";
import type { Page } from "../layouts/MainLayout";
import type { Instance, ClawType } from "../types";

// ---- Static SVG icons ----
const icons: Record<string, () => JSX.Element> = {
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
      <rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  ),
  sandbox: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" />
      <circle cx="6" cy="6" r="1" fill="currentColor" /><circle cx="6" cy="18" r="1" fill="currentColor" />
      <line x1="10" y1="6" x2="18" y2="6" /><line x1="10" y1="18" x2="18" y2="18" />
    </svg>
  ),
  settings: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  ),
  // Folder icon — like iPhone app folder, for "more claws"
  folder: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="w-5 h-5">
      <rect x="3" y="4" width="8" height="8" rx="2" fill="currentColor" opacity="0.15" stroke="none" />
      <rect x="13" y="4" width="8" height="8" rx="2" fill="currentColor" opacity="0.15" stroke="none" />
      <rect x="3" y="14" width="8" height="6" rx="2" fill="currentColor" opacity="0.15" stroke="none" />
      <rect x="13" y="14" width="8" height="6" rx="2" fill="currentColor" opacity="0.15" stroke="none" />
      <rect x="4.5" y="5.5" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1" />
      <rect x="14.5" y="5.5" width="5" height="5" rx="1" stroke="currentColor" stroke-width="1" />
      <rect x="4.5" y="15" width="5" height="3.5" rx="1" stroke="currentColor" stroke-width="1" />
      <rect x="14.5" y="15" width="5" height="3.5" rx="1" stroke="currentColor" stroke-width="1" />
    </svg>
  ),
  user: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" /><circle cx="12" cy="7" r="4" />
    </svg>
  ),
};

function ClawLogo(props: { logo: string; size?: string }) {
  const sz = () => props.size || "text-lg";
  return <span class={`${sz()} leading-none`}>{props.logo || "📦"}</span>;
}

/**
 * IconBar rules:
 *  - Only claw types that have at least one instance are shown
 *  - "openclaw" is always pinned at slot 1 (if it has instances)
 *  - Slots 2-3: most recently visited claw types (tracked via lastVisited)
 *  - Slot 4 (if more exist): folder icon → opens dialog listing all claw instances
 *  - No "add" button in IconBar
 */
export default function IconBar(props: {
  activePage: Page;
  onNavigate: (page: Page) => void;
  clawTypes: ClawType[];
  instances: Instance[];
}) {
  const [showMoreDialog, setShowMoreDialog] = createSignal(false);
  // Track last-visited claw types (most recent first, excluding openclaw)
  const [recentClawIds, setRecentClawIds] = createSignal<string[]>([]);

  // All claw types that have at least one instance
  const clawsWithInstances = () => {
    const types = new Set(props.instances.map((i) => i.claw_type));
    return props.clawTypes.filter((t) => types.has(t.id));
  };

  // Visible slots (max 3): openclaw first, then 2 recent
  const visibleClaws = () => {
    const all = clawsWithInstances();
    if (all.length === 0) return [];

    const result: ClawType[] = [];

    // Slot 1: openclaw (if has instances)
    const oc = all.find((t) => t.id === "openclaw");
    if (oc) result.push(oc);

    // Slots 2-3: most recently visited (that have instances, excluding openclaw)
    const recent = recentClawIds();
    for (const rid of recent) {
      if (result.length >= 3) break;
      if (rid === "openclaw") continue;
      const t = all.find((c) => c.id === rid);
      if (t && !result.find((r) => r.id === t.id)) result.push(t);
    }

    // Fill remaining slots with any other claw that has instances
    for (const t of all) {
      if (result.length >= 3) break;
      if (!result.find((r) => r.id === t.id)) result.push(t);
    }

    return result;
  };

  // Overflow: claw types with instances that are NOT in the visible 3
  const overflowClaws = () => {
    const visible = new Set(visibleClaws().map((t) => t.id));
    return clawsWithInstances().filter((t) => !visible.has(t.id));
  };

  // Track navigation to update recents
  const handleNavigate = (page: Page) => {
    if (page.startsWith("claw:")) {
      const clawId = page.slice(5);
      if (clawId !== "openclaw") {
        setRecentClawIds((prev) => [clawId, ...prev.filter((id) => id !== clawId)].slice(0, 10));
      }
    }
    setShowMoreDialog(false);
    props.onNavigate(page);
  };

  function NavBtn(p: { page: Page; label: string; icon: () => JSX.Element }) {
    const active = () => props.activePage === p.page;
    return (
      <button
        class={`relative w-10 h-10 rounded-lg flex items-center justify-center transition-colors ${
          active()
            ? "bg-gray-700 text-white"
            : "text-gray-400 hover:text-white hover:bg-gray-800"
        }`}
        onClick={() => handleNavigate(p.page)}
        title={p.label}
      >
        {active() && <div class="absolute left-0 top-2 bottom-2 w-0.5 bg-white rounded-r" />}
        {p.icon()}
      </button>
    );
  }

  function ClawBtn(p: { claw: ClawType }) {
    const page: Page = `claw:${p.claw.id}`;
    const count = () => props.instances.filter((i) => i.claw_type === p.claw.id).length;
    return (
      <div class="relative">
        <NavBtn page={page} label={p.claw.display_name} icon={() => <ClawLogo logo={p.claw.logo} />} />
        <Show when={count() > 1}>
          <span class="absolute -top-0.5 -right-0.5 bg-blue-600 text-[9px] text-white rounded-full w-3.5 h-3.5 flex items-center justify-center">
            {count()}
          </span>
        </Show>
      </div>
    );
  }

  return (
    <>
      <nav class="flex flex-col w-14 bg-gray-950 border-r border-gray-800 items-center py-3 shrink-0">
        {/* Logo */}
        <button
          class="w-10 h-10 rounded-lg flex items-center justify-center hover:bg-gray-800 transition-colors"
          onClick={() => handleNavigate("home")}
          title="ClawEnv"
        >
          <span class="w-7 h-7 block">{icons.logo()}</span>
        </button>

        <div class="w-8 border-t border-gray-700 my-2" />

        {/* Home */}
        <NavBtn page="home" label="Home" icon={icons.home} />

        <div class="w-8 border-t border-gray-700 my-2" />

        {/* Claw type icons: max 3 visible + folder overflow */}
        <div class="flex flex-col items-center gap-1 flex-1">
          <For each={visibleClaws()}>
            {(claw) => <ClawBtn claw={claw} />}
          </For>

          {/* Folder icon: shown when there are overflow claws */}
          <Show when={overflowClaws().length > 0}>
            <button
              class={`w-10 h-10 rounded-lg flex items-center justify-center transition-colors ${
                showMoreDialog()
                  ? "bg-gray-700 text-white"
                  : "text-gray-500 hover:text-white hover:bg-gray-800"
              }`}
              onClick={() => setShowMoreDialog(!showMoreDialog())}
              title="More claw types"
            >
              {icons.folder()}
            </button>
          </Show>
        </div>

        {/* Bottom nav */}
        <div class="flex flex-col items-center gap-1">
          <NavBtn page="sandbox" label="Sandbox" icon={icons.sandbox} />
          <NavBtn page="settings" label="Settings" icon={icons.settings} />
          <button
            class="w-10 h-10 rounded-lg flex items-center justify-center text-gray-600 cursor-not-allowed opacity-40"
            title="User login (coming soon)"
            disabled
          >
            {icons.user()}
          </button>
        </div>
      </nav>

      {/* "More claws" folder dialog */}
      <Show when={showMoreDialog()}>
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowMoreDialog(false)}>
          <div class="bg-gray-800 rounded-xl p-4 w-72 max-h-96 overflow-y-auto shadow-xl" onClick={(e) => e.stopPropagation()}>
            <h3 class="text-sm font-semibold text-gray-300 mb-3">All Claw Instances</h3>
            <div class="space-y-1">
              <For each={overflowClaws()}>
                {(claw) => {
                  const count = () => props.instances.filter((i) => i.claw_type === claw.id).length;
                  return (
                    <button
                      class="w-full flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-gray-700 transition-colors text-left"
                      onClick={() => handleNavigate(`claw:${claw.id}`)}
                    >
                      <ClawLogo logo={claw.logo} />
                      <span class="flex-1 text-sm text-white">{claw.display_name}</span>
                      <span class="text-xs text-gray-400">{count()}</span>
                    </button>
                  );
                }}
              </For>
            </div>
          </div>
        </div>
      </Show>
    </>
  );
}

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
  add: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" class="w-5 h-5">
      <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="16" /><line x1="8" y1="12" x2="16" y2="12" />
    </svg>
  ),
  more: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" class="w-5 h-5">
      <circle cx="12" cy="5" r="1.5" fill="currentColor" />
      <circle cx="12" cy="12" r="1.5" fill="currentColor" />
      <circle cx="12" cy="19" r="1.5" fill="currentColor" />
    </svg>
  ),
  user: () => (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="w-5 h-5">
      <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" /><circle cx="12" cy="7" r="4" />
    </svg>
  ),
};

// ---- Claw logo renderer (emoji or fallback) ----
function ClawLogo(props: { logo: string; size?: string }) {
  const sz = () => props.size || "text-lg";
  return <span class={`${sz()} leading-none`}>{props.logo || "📦"}</span>;
}

export default function IconBar(props: {
  activePage: Page;
  onNavigate: (page: Page) => void;
  clawTypes: ClawType[];
  instances: Instance[];
  pinnedClawIds: string[];
  onAddInstance: (clawType?: string) => void;
}) {
  const [showMoreDialog, setShowMoreDialog] = createSignal(false);
  const [showAddDialog, setShowAddDialog] = createSignal(false);

  // Pinned claw types (always visible, order from pinnedClawIds)
  const pinnedClaws = () =>
    props.pinnedClawIds
      .map((id) => props.clawTypes.find((t) => t.id === id))
      .filter((t): t is ClawType => !!t);

  // Unpinned claw types that have at least one instance
  const unpinnedWithInstances = () => {
    const pinnedSet = new Set(props.pinnedClawIds);
    const typesWithInstances = new Set(props.instances.map((i) => i.claw_type));
    return props.clawTypes.filter(
      (t) => !pinnedSet.has(t.id) && typesWithInstances.has(t.id)
    );
  };

  // The currently active claw type if it's unpinned (show below "more")
  const activeUnpinnedClaw = () => {
    const page = props.activePage;
    if (!page.startsWith("claw:")) return null;
    const clawId = page.slice(5);
    if (props.pinnedClawIds.includes(clawId)) return null;
    return props.clawTypes.find((t) => t.id === clawId) || null;
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
        onClick={() => p.page && props.onNavigate(p.page)}
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
        <Show when={count() > 0}>
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
          onClick={() => props.onNavigate("home")}
          title="ClawEnv"
        >
          <span class="w-7 h-7 block">{icons.logo()}</span>
        </button>

        <div class="w-8 border-t border-gray-700 my-2" />

        {/* Home + Add button */}
        <div class="flex flex-col items-center gap-1">
          <NavBtn page="home" label="Home" icon={icons.home} />
          <button
            class="w-10 h-10 rounded-lg flex items-center justify-center text-green-400 hover:text-green-300 hover:bg-gray-800 transition-colors"
            onClick={() => setShowAddDialog(true)}
            title="Add new claw instance"
          >
            {icons.add()}
          </button>
        </div>

        <div class="w-8 border-t border-gray-700 my-2" />

        {/* Pinned claw types */}
        <div class="flex flex-col items-center gap-1 flex-1 overflow-y-auto">
          <For each={pinnedClaws()}>
            {(claw) => <ClawBtn claw={claw} />}
          </For>

          {/* "More" button — shown when there are unpinned claws with instances */}
          <Show when={unpinnedWithInstances().length > 0}>
            <button
              class={`w-10 h-10 rounded-lg flex items-center justify-center transition-colors ${
                showMoreDialog() ? "bg-gray-700 text-white" : "text-gray-500 hover:text-white hover:bg-gray-800"
              }`}
              onClick={() => setShowMoreDialog(!showMoreDialog())}
              title="More claw types"
            >
              {icons.more()}
            </button>
          </Show>

          {/* Active unpinned claw — always visible below "more" */}
          <Show when={activeUnpinnedClaw()}>
            {(claw) => <ClawBtn claw={claw()} />}
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

      {/* "More claws" dialog */}
      <Show when={showMoreDialog()}>
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowMoreDialog(false)}>
          <div class="bg-gray-800 rounded-xl p-4 w-72 max-h-96 overflow-y-auto shadow-xl" onClick={(e) => e.stopPropagation()}>
            <h3 class="text-sm font-semibold text-gray-300 mb-3">Other Claw Instances</h3>
            <div class="space-y-1">
              <For each={unpinnedWithInstances()}>
                {(claw) => {
                  const count = () => props.instances.filter((i) => i.claw_type === claw.id).length;
                  return (
                    <button
                      class="w-full flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-gray-700 transition-colors text-left"
                      onClick={() => { props.onNavigate(`claw:${claw.id}`); setShowMoreDialog(false); }}
                    >
                      <ClawLogo logo={claw.logo} />
                      <span class="flex-1 text-sm text-white">{claw.display_name}</span>
                      <span class="text-xs text-gray-400">{count()} instance{count() > 1 ? "s" : ""}</span>
                    </button>
                  );
                }}
              </For>
            </div>
          </div>
        </div>
      </Show>

      {/* "Add new instance" dialog — choose claw type */}
      <Show when={showAddDialog()}>
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowAddDialog(false)}>
          <div class="bg-gray-800 rounded-xl p-5 w-80 max-h-[80vh] overflow-y-auto shadow-xl" onClick={(e) => e.stopPropagation()}>
            <h3 class="text-base font-semibold text-white mb-1">Add New Instance</h3>
            <p class="text-xs text-gray-400 mb-4">Choose a claw type to install</p>
            <div class="space-y-1">
              <For each={props.clawTypes}>
                {(claw) => (
                  <button
                    class="w-full flex items-center gap-3 px-3 py-2.5 rounded-lg hover:bg-gray-700 transition-colors text-left"
                    onClick={() => { props.onAddInstance(claw.id); setShowAddDialog(false); }}
                  >
                    <ClawLogo logo={claw.logo} />
                    <div class="flex-1">
                      <div class="text-sm text-white">{claw.display_name}</div>
                      <div class="text-[10px] text-gray-500">{claw.npm_package}</div>
                    </div>
                  </button>
                )}
              </For>
            </div>
          </div>
        </div>
      </Show>
    </>
  );
}

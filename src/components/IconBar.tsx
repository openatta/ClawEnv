type Page = "home" | "openclaw" | "settings";

export default function IconBar(props: {
  activePage: Page;
  onNavigate: (page: Page) => void;
}) {
  const items: { id: Page; label: string; icon: string; position: "top" | "bottom" }[] = [
    { id: "home", label: "Home", icon: "H", position: "top" },
    { id: "openclaw", label: "OpenClaw", icon: "OC", position: "top" },
    { id: "settings", label: "Settings", icon: "S", position: "bottom" },
  ];

  const topItems = items.filter((i) => i.position === "top");
  const bottomItems = items.filter((i) => i.position === "bottom");

  return (
    <nav class="flex flex-col w-14 bg-gray-950 border-r border-gray-800 items-center py-3 shrink-0">
      {/* Logo */}
      <button
        class="w-9 h-9 rounded-lg bg-indigo-600 flex items-center justify-center text-xs font-bold mb-4 hover:bg-indigo-500 transition-colors"
        onClick={() => props.onNavigate("home")}
        title="ClawEnv"
      >
        CE
      </button>

      {/* Top items */}
      <div class="flex flex-col items-center gap-1 flex-1">
        {topItems.map((item) => (
          <button
            class={`relative w-10 h-10 rounded-lg flex items-center justify-center text-xs font-medium transition-colors ${
              props.activePage === item.id
                ? "bg-gray-700 text-white"
                : "text-gray-400 hover:text-white hover:bg-gray-800"
            }`}
            onClick={() => props.onNavigate(item.id)}
            title={item.label}
          >
            {/* Active indicator */}
            {props.activePage === item.id && (
              <div class="absolute left-0 top-2 bottom-2 w-0.5 bg-white rounded-r" />
            )}
            {item.icon}
          </button>
        ))}
      </div>

      {/* Bottom items */}
      <div class="flex flex-col items-center gap-1">
        {bottomItems.map((item) => (
          <button
            class={`relative w-10 h-10 rounded-lg flex items-center justify-center text-xs font-medium transition-colors ${
              props.activePage === item.id
                ? "bg-gray-700 text-white"
                : "text-gray-400 hover:text-white hover:bg-gray-800"
            }`}
            onClick={() => props.onNavigate(item.id)}
            title={item.label}
          >
            {props.activePage === item.id && (
              <div class="absolute left-0 top-2 bottom-2 w-0.5 bg-white rounded-r" />
            )}
            {item.icon}
          </button>
        ))}
        {/* User placeholder (greyed out) */}
        <button
          class="w-10 h-10 rounded-lg flex items-center justify-center text-xs text-gray-600 cursor-not-allowed"
          title="User login (coming soon)"
          disabled
        >
          U
        </button>
      </div>
    </nav>
  );
}

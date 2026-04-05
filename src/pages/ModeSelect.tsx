import { invoke } from "@tauri-apps/api/core";

export default function ModeSelect(props: { onComplete: () => void }) {
  async function selectMode(mode: "general" | "developer") {
    try {
      await invoke("create_default_config", { userMode: mode });
    } catch (e) {
      console.error("Failed to create config:", e);
    }
    props.onComplete();
  }

  return (
    <div class="flex h-screen items-center justify-center bg-gray-900 text-white">
      <div class="text-center max-w-2xl px-8">
        <h1 class="text-3xl font-bold mb-2">ClawEnv</h1>
        <p class="text-gray-400 mb-10">OpenClaw Installation & Management Tool</p>

        <p class="text-sm text-gray-400 mb-8">
          Choose your usage mode (can be changed in Settings):
        </p>

        <div class="grid grid-cols-2 gap-6">
          <button
            class="bg-gray-800 border border-gray-700 rounded-xl p-6 text-left hover:border-indigo-500 transition-colors group"
            onClick={() => selectMode("general")}
          >
            <div class="text-lg font-medium mb-3 group-hover:text-indigo-400">
              Normal User
            </div>
            <ul class="text-sm text-gray-400 space-y-1.5">
              <li>- Guided installation wizard</li>
              <li>- Automatic sandbox (secure)</li>
              <li>- One-click upgrade</li>
              <li>- No technical knowledge required</li>
            </ul>
          </button>

          <button
            class="bg-gray-800 border border-gray-700 rounded-xl p-6 text-left hover:border-indigo-500 transition-colors group"
            onClick={() => selectMode("developer")}
          >
            <div class="text-lg font-medium mb-3 group-hover:text-indigo-400">
              Developer
            </div>
            <ul class="text-sm text-gray-400 space-y-1.5">
              <li>- Full CLI tools</li>
              <li>- Multi-instance management</li>
              <li>- Native or sandbox mode</li>
              <li>- Skill development scaffold</li>
              <li>- Snapshot & rollback</li>
            </ul>
          </button>
        </div>
      </div>
    </div>
  );
}

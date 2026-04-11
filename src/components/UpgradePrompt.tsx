import { createSignal } from "solid-js";
import type { Instance } from "../types";

type Props = {
  instances: Instance[];
  onSkip: () => void;
  onUpgraded: (instances: Instance[]) => void;
};

export default function UpgradePrompt(props: Props) {
  const [upgrading, setUpgrading] = createSignal(false);

  // Collect unique claw display names from instances
  const clawNames = () => {
    const names = new Set(props.instances.map((i) => i.display_name || i.claw_type || "Claw"));
    return Array.from(names).join(", ");
  };

  return (
    <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div class="bg-gray-800 border border-gray-700 rounded-xl p-6 max-w-md w-full shadow-2xl">
        <h2 class="text-lg font-bold mb-1">Update Available</h2>
        <p class="text-sm text-gray-400 mb-4">
          A newer version of {clawNames()} is available for your instances.
        </p>

        <div class="bg-gray-900 rounded-lg p-3 mb-4 text-sm">
          {props.instances.map((inst) => (
            <div class="flex justify-between py-1">
              <span class="flex items-center gap-2">
                <span>{inst.logo}</span>
                <span>{inst.name}</span>
              </span>
              <span class="text-gray-400">v{inst.version}</span>
            </div>
          ))}
        </div>

        <div class="flex justify-end gap-3">
          <button
            class="px-4 py-2 text-sm bg-gray-700 hover:bg-gray-600 rounded"
            onClick={props.onSkip}
            disabled={upgrading()}
          >
            Skip
          </button>
          <button
            class="px-4 py-2 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
            disabled={upgrading()}
            onClick={() => { props.onSkip(); }}
          >
            {upgrading() ? "Upgrading..." : "Update Now"}
          </button>
        </div>

        <label class="flex items-center gap-2 mt-4 text-xs text-gray-500 cursor-pointer">
          <input type="checkbox" class="rounded" />
          Auto-update in the future
        </label>
      </div>
    </div>
  );
}

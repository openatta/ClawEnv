import { createSignal } from "solid-js";

type Props = {
  novncUrl: string;
  onClose: () => void;
};

export default function NoVncPanel(props: Props) {
  const [fullscreen, setFullscreen] = createSignal(false);

  return (
    <div class={`${fullscreen() ? "fixed inset-0 z-50" : "relative"} bg-gray-950 border border-gray-700 rounded-lg overflow-hidden`}>
      {/* Header bar */}
      <div class="flex items-center justify-between px-3 py-2 bg-gray-900 border-b border-gray-700">
        <div class="flex items-center gap-2">
          <div class="w-2 h-2 rounded-full bg-orange-500 animate-pulse" />
          <span class="text-sm font-medium text-orange-400">Human intervention required</span>
        </div>
        <div class="flex items-center gap-2">
          <button
            class="px-2 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded"
            onClick={() => setFullscreen(!fullscreen())}
          >
            {fullscreen() ? "Exit Fullscreen" : "Fullscreen"}
          </button>
          <button
            class="px-2 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded"
            onClick={() => window.open(props.novncUrl, "_blank")}
          >
            ↗ New Window
          </button>
          <button
            class="px-3 py-1 text-xs bg-indigo-600 hover:bg-indigo-500 rounded"
            onClick={props.onClose}
          >
            Continue Auto
          </button>
        </div>
      </div>
      {/* noVNC iframe */}
      <iframe
        src={props.novncUrl}
        class="w-full border-0"
        style={fullscreen() ? "height: calc(100vh - 44px)" : "height: 480px"}
        title="noVNC Remote Browser"
      />
    </div>
  );
}

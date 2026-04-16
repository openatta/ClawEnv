import { Show, For } from "solid-js";

export default function LogBox(p: { logs: string[]; height?: string }) {
  let ref_el: HTMLDivElement | undefined;
  // Auto-scroll to bottom when logs change
  const scrollToBottom = () => {
    if (ref_el) ref_el.scrollTop = ref_el.scrollHeight;
  };
  return (
    <div ref={ref_el} class={`bg-gray-950 rounded border border-gray-700 p-2 overflow-y-auto font-mono text-xs text-gray-400 ${p.height || "h-40"}`}>
      <For each={p.logs}>
        {(line) => {
          // Schedule scroll after render
          setTimeout(scrollToBottom, 10);
          return <div class={
            line.includes("ERR!") || line.includes("✗ ERROR") || (line.includes("fail") && !line.includes("optional")) ? "text-red-400"
            : line.includes("✓") || line.includes("OK") || line.includes("done") ? "text-green-400"
            : line.startsWith("---") ? "text-gray-600"
            : ""
          }>{line}</div>;
        }}
      </For>
      <Show when={p.logs.length === 0}><span class="text-gray-600">Waiting...</span></Show>
    </div>
  );
}

import { createSignal, onMount, Show } from "solid-js";
import { t } from "../i18n";

type Props = {
  operation: "start" | "stop" | "restart";
  instanceName: string;
  onComplete: () => void;
  doAction: () => Promise<void>;
};

const LABELS = {
  start: { zh: "启动", en: "Starting" },
  stop: { zh: "停止", en: "Stopping" },
  restart: { zh: "重启", en: "Restarting" },
};

const TIMEOUTS = { start: 15, stop: 5, restart: 20 };

export default function OperationModal(props: Props) {
  const [elapsed, setElapsed] = createSignal(0);
  const [done, setDone] = createSignal(false);
  const [error, setError] = createSignal("");
  const total = () => TIMEOUTS[props.operation];
  const label = () => LABELS[props.operation];

  onMount(async () => {
    const timer = setInterval(() => setElapsed(v => v + 1), 1000);

    try {
      await props.doAction();
      setDone(true);
      setTimeout(() => props.onComplete(), 800);
    } catch (e) {
      setError(String(e));
    } finally {
      clearInterval(timer);
    }
  });

  const remaining = () => Math.max(0, total() - elapsed());
  const pct = () => Math.min(100, (elapsed() / total()) * 100);

  return (
    <div class="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div class="bg-gray-800 border border-gray-700 rounded-xl p-5 w-80 shadow-2xl">
        <Show when={!done() && !error()}>
          <h3 class="text-base font-bold mb-3">
            {t(label().zh, label().en)} '{props.instanceName}'...
          </h3>
          <div class="w-full bg-gray-700 rounded-full h-1.5 mb-2">
            <div class="bg-indigo-500 h-1.5 rounded-full transition-all"
              style={{ width: `${pct()}%` }} />
          </div>
          <p class="text-xs text-gray-400">
            {t(`预计还需 ${remaining()} 秒`, `~${remaining()}s remaining`)}
          </p>
        </Show>
        <Show when={done()}>
          <div class="text-center">
            <div class="text-green-400 text-lg mb-1">✓</div>
            <p class="text-sm text-green-400">{t("操作完成", "Done")}</p>
          </div>
        </Show>
        <Show when={error()}>
          <h3 class="text-base font-bold mb-2 text-red-400">{t("操作失败", "Failed")}</h3>
          <p class="text-xs text-red-300 mb-3">{error()}</p>
          <button class="px-3 py-1.5 text-sm bg-gray-700 hover:bg-gray-600 rounded w-full"
            onClick={props.onComplete}>{t("关闭", "Close")}</button>
        </Show>
      </div>
    </div>
  );
}

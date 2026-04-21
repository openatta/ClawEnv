import { Show } from "solid-js";
import { t } from "../../../i18n";

/** Lite-only step: after the claw is installed + running, tell the user
 *  where to go configure their provider API key. ClawEnv itself never
 *  handles credentials (v0.3.0 contract) — each claw has its own Web
 *  UI that owns the credential UX. Lite shows a short, claw-specific
 *  reminder so first-time users aren't left wondering.
 *
 *  Per user direction we show ONLY the Web-UI path, not the CLI /
 *  .env / env-var alternatives — those exist but surfacing them would
 *  clutter the Lite "happy path".
 */
export default function LiteStepApiKeyHint(props: {
  clawType: string;          // "openclaw" | "hermes" | ...
  clawDisplayName: string;   // e.g. "OpenClaw"
  onFinish: () => void;
}) {
  const isOpenClaw = () => props.clawType === "openclaw";
  const isHermes   = () => props.clawType === "hermes";

  return (
    <div class="flex flex-col h-full">
      <h2 class="text-xl font-bold mb-2">
        {t("配置 API Key", "Configure Your API Key")}
      </h2>
      <p class="text-sm text-gray-400 mb-4">
        {t(
          `${props.clawDisplayName} 已安装完成，接下来需要配置你的 AI 服务商 API Key 才能使用。`,
          `${props.clawDisplayName} is installed. Next, configure your AI provider's API key before you can use it.`,
        )}
      </p>

      <div class="flex-1 overflow-y-auto">
        <Show when={isOpenClaw()}>
          <div class="bg-gray-800 border border-gray-700 rounded-lg p-4 space-y-3">
            <div class="text-sm font-medium text-indigo-300">
              {t("通过 OpenClaw 控制面板配置", "Configure via the OpenClaw Control Panel")}
            </div>
            <ol class="list-decimal list-inside space-y-2 text-sm text-gray-300">
              <li>
                {t(
                  "完成后进入主界面，点击",
                  "Once on the main page, click",
                )}
                {" "}
                <span class="px-1.5 py-0.5 rounded bg-gray-700 text-xs font-mono">
                  {t("打开控制面板", "Open Control Panel")}
                </span>
                {" "}
                {t("打开 OpenClaw 的 Web 管理界面。", "to open OpenClaw's Web UI.")}
              </li>
              <li>
                {t(
                  "在左侧菜单选择",
                  "In the left sidebar, go to",
                )}
                {" "}
                <span class="font-mono text-xs">Settings → Models → Providers</span>
                {"。"}
              </li>
              <li>
                {t(
                  "找到你使用的 provider（如 OpenAI / Anthropic / DeepSeek），粘贴你的 apiKey，保存。",
                  "Find the provider you use (OpenAI / Anthropic / DeepSeek / ...), paste your apiKey, save.",
                )}
              </li>
            </ol>
          </div>
        </Show>

        <Show when={isHermes()}>
          <div class="bg-gray-800 border border-gray-700 rounded-lg p-4 space-y-3">
            <div class="text-sm font-medium text-indigo-300">
              {t("通过 Hermes Dashboard 配置", "Configure via the Hermes Dashboard")}
            </div>
            <ol class="list-decimal list-inside space-y-2 text-sm text-gray-300">
              <li>
                {t(
                  "完成后进入主界面，点击",
                  "Once on the main page, click",
                )}
                {" "}
                <span class="px-1.5 py-0.5 rounded bg-gray-700 text-xs font-mono">
                  {t("打开控制面板", "Open Control Panel")}
                </span>
                {" "}
                {t("打开 Hermes Dashboard。", "to open the Hermes Dashboard.")}
              </li>
              <li>
                {t(
                  "在顶部菜单进入",
                  "At the top, go to",
                )}
                {" "}
                <span class="font-mono text-xs">Settings → LLM Providers</span>
                {"。"}
              </li>
              <li>
                {t(
                  "填入 API Key，保存。",
                  "Paste your API key and save.",
                )}
              </li>
            </ol>
          </div>
        </Show>

        {/* Fallback catch-all for any other claw we ship later. Keeps lite
            functioning without a code change — the wording is generic
            enough to not mislead. */}
        <Show when={!isOpenClaw() && !isHermes()}>
          <div class="bg-gray-800 border border-gray-700 rounded-lg p-4 space-y-3">
            <p class="text-sm text-gray-300">
              {t(
                "完成后进入主界面，点击「打开控制面板」打开 " +
                props.clawDisplayName +
                " 的 Web 管理界面，在设置页中填入你的 API Key 并保存。",
                `Once on the main page, click "Open Control Panel" to open ${props.clawDisplayName}'s web UI, then enter your API key in the settings page and save.`,
              )}
            </p>
          </div>
        </Show>

        <div class="mt-4 text-xs text-gray-500">
          {t(
            "ClawEnv Lite 不会经手你的任何 API Key —— 凭证完全保存在 " + props.clawDisplayName + " 自己的存储中。",
            `ClawEnv Lite never handles your API key — credentials live entirely in ${props.clawDisplayName}'s own storage.`,
          )}
        </div>
      </div>

      <div class="flex justify-end pt-3 border-t border-gray-800 shrink-0">
        <button class="px-4 py-1.5 text-sm bg-green-600 hover:bg-green-500 rounded"
          onClick={props.onFinish}>
          {t("进入主界面", "Go to Main Page")}
        </button>
      </div>
    </div>
  );
}

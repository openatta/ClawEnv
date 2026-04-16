import { createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

export default function StepInstallPlan(props: {
  lang: "zh-CN" | "en";
  installMethod: () => string;
  onMethodChange: (m: string) => void;
  localFilePath: () => string;
  onFilePathChange: (p: string) => void;
  installBrowser: () => boolean;
  onBrowserChange: (v: boolean) => void;
  installMcpBridge: () => boolean;
  onMcpBridgeChange: (v: boolean) => void;
  clawDisplayName: string;
}) {
  const zh = () => props.lang === "zh-CN";
  const [fileValidation, setFileValidation] = createSignal<{ valid: boolean; error: string } | null>(null);
  const [hasNative, setHasNative] = createSignal(false);

  // Check if native instance exists
  invoke<boolean>("has_native_instance").then(v => setHasNative(v)).catch(() => {});

  async function pickFile() {
    try {
      const path = await invoke<string>("pick_import_file");
      props.onFilePathChange(path);
      const result = await invoke<{ valid: boolean; error: string; is_native: boolean }>("validate_import_file", { filePath: path });
      setFileValidation(result);
    } catch { /* cancelled */ }
  }

  return (
    <div>
      <h2 class="text-xl font-bold mb-4">{zh() ? "安装方案" : "Installation Plan"}</h2>

      {/* Install Mode group */}
      <fieldset class="border border-gray-600 rounded-lg p-4 mb-4">
        <legend class="px-2 text-sm font-medium text-gray-300">{zh() ? "安装模式" : "Install Mode"}</legend>
        <div class="space-y-2">
          <label class="flex items-center gap-3 p-2.5 rounded border border-gray-700 cursor-pointer hover:border-gray-500">
            <input type="radio" name="im" checked={props.installMethod() === "online"} onChange={() => props.onMethodChange("online")} class="w-4 h-4 shrink-0" />
            <div>
              <div class="font-medium text-sm">{zh() ? "沙盒 - 在线构建" : "Sandbox - Online Build"}</div>
              <div class="text-xs text-gray-400">{zh() ? "创建虚拟机/容器并从源安装（推荐）" : "Create VM/container and install from source (recommended)"}</div>
            </div>
          </label>
          <label class="flex items-center gap-3 p-2.5 rounded border border-gray-700 cursor-pointer hover:border-gray-500">
            <input type="radio" name="im" checked={props.installMethod() === "local"} onChange={() => props.onMethodChange("local")} class="w-4 h-4 shrink-0" />
            <div>
              <div class="font-medium text-sm">{zh() ? "沙盒 - 本地镜像" : "Sandbox - Local Image"}</div>
              <div class="text-xs text-gray-400">{zh() ? "导入预构建的沙盒镜像文件" : "Import a pre-built sandbox image file"}</div>
            </div>
          </label>
          <label class={`flex items-center gap-3 p-2.5 rounded border border-dashed border-yellow-700/50 ${hasNative() ? "opacity-40 cursor-not-allowed" : "cursor-pointer hover:border-yellow-600/50"}`}>
            <input type="radio" name="im" checked={props.installMethod() === "native"} disabled={hasNative()}
              onChange={() => props.onMethodChange("native")} class="w-4 h-4 shrink-0" />
            <div>
              <div class="font-medium text-sm">{zh() ? "本地 - 在线安装" : "Native - Online Install"}</div>
              <div class="text-xs text-gray-400">{zh() ? "直接安装在本机 — 无需虚拟机" : "Install directly on this machine — no VM"}</div>
              {hasNative() && <div class="text-xs text-red-400 mt-1">{zh() ? "已有本地实例，不能重复安装" : "Native instance already exists"}</div>}
            </div>
          </label>
          <label class={`flex items-center gap-3 p-2.5 rounded border border-dashed border-yellow-700/50 ${hasNative() ? "opacity-40 cursor-not-allowed" : "cursor-pointer hover:border-yellow-600/50"}`}>
            <input type="radio" name="im" checked={props.installMethod() === "native-import"} disabled={hasNative()}
              onChange={() => { props.onMethodChange("native-import"); props.onFilePathChange(""); setFileValidation(null); }} class="w-4 h-4 shrink-0" />
            <div>
              <div class="font-medium text-sm">{zh() ? "本地 - 导入 Bundle" : "Native - Import Bundle"}</div>
              <div class="text-xs text-gray-400">{zh() ? "从导出的离线包导入" : "Import from exported offline bundle"}</div>
              {hasNative() && <div class="text-xs text-red-400 mt-1">{zh() ? "已有本地实例，不能重复安装" : "Native instance already exists"}</div>}
            </div>
          </label>
        </div>
        <Show when={props.installMethod() === "local" || props.installMethod() === "native-import"}>
          <div class="mt-3 flex gap-2">
            <input type="text" placeholder={props.installMethod() === "local" ? "sandbox-image.tar.gz" : "native-bundle.tar.gz"}
              value={props.localFilePath()} onInput={e => { props.onFilePathChange(e.currentTarget.value); setFileValidation(null); }}
              class="bg-gray-800 border border-gray-600 rounded px-3 py-2 flex-1 text-sm" />
            <button class="px-3 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-sm shrink-0"
              onClick={pickFile}>Browse</button>
          </div>
          <Show when={fileValidation()}>
            {fileValidation()!.valid
              ? <div class="text-xs text-green-400 mt-1">✓ File validated — compatible with this platform</div>
              : <div class="text-xs text-red-400 mt-1">✗ {fileValidation()!.error}</div>
            }
          </Show>
        </Show>
      </fieldset>

      {/* Optional components group */}
      <fieldset class="border border-gray-600 rounded-lg p-4">
        <legend class="px-2 text-sm font-medium text-gray-300">{zh() ? "可选组件" : "Optional Components"}</legend>
        <div class="space-y-3">
          {/* MCP Bridge Plugin — default ON */}
          <label class="flex items-start gap-3 p-2.5 rounded border border-green-700/30 bg-gray-800/50 cursor-pointer hover:border-green-600/40">
            <input type="checkbox" checked={props.installMcpBridge()} onChange={e => props.onMcpBridgeChange(e.currentTarget.checked)}
              class="w-4 h-4 mt-0.5 shrink-0" />
            <div>
              <div class="text-sm font-medium">MCP Bridge Plugin <span class="text-green-400 text-xs">({zh() ? "推荐" : "recommended"})</span></div>
              <div class="text-xs text-gray-400 mt-1">
                {zh()
                  ? `使 ${props.clawDisplayName} Agent 能通过安全的权限控制桥接访问宿主机的文件、命令和工具`
                  : `Enables ${props.clawDisplayName} agents to access host machine files, commands, and tools through a secure, permission-controlled bridge.`}
              </div>
              {!props.installMcpBridge() && (
                <div class="text-xs text-yellow-500 mt-1">
                  {zh() ? "⚠ 不安装此插件，Agent 将无法访问宿主机上的程序和数据" : "⚠ Without this plugin, agents cannot access programs or data on your host machine."}
                </div>
              )}
            </div>
          </label>

          {/* Browser Automation — default OFF */}
          <label class={`flex items-start gap-3 p-2.5 rounded border border-gray-700 bg-gray-800/50 ${props.installMethod() !== "online" ? "opacity-40 cursor-not-allowed" : "cursor-pointer hover:border-gray-500"}`}>
            <input type="checkbox" checked={props.installBrowser() && props.installMethod() === "online"} disabled={props.installMethod() !== "online"}
              onChange={e => props.onBrowserChange(e.currentTarget.checked)}
              class="w-4 h-4 mt-0.5 shrink-0" />
            <div>
              <div class="text-sm font-medium">{zh() ? "浏览器自动化（Chromium Headless）" : "Browser Automation (Chromium Headless)"}</div>
              <div class="text-xs text-gray-400 mt-1">
                {zh()
                  ? "用于网页抓取、截图、CDP 自动化和验证码处理"
                  : "Required for web scraping, screenshots, CDP automation, and CAPTCHA handling."}
              </div>
              <div class="text-xs text-yellow-500 mt-1">
                {zh() ? "⚠ 增加约 630MB 空间，可稍后在设置中安装" : "⚠ Adds ~630MB. Can be installed later from Settings."}
              </div>
            </div>
          </label>
        </div>
      </fieldset>
    </div>
  );
}

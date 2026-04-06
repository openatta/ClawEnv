import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

const i18n = {
  "zh-CN": {
    title: "ClawEnv",
    subtitle: "OpenClaw 安装与管理工具",
    desc: "ClawEnv 为 OpenClaw AI Agent 框架提供跨平台的安全沙盒安装、升级与管理。支持 macOS (Lima)、Windows (WSL2)、Linux (Podman) 三平台对等架构。",
    choose: "请选择您的使用方式（可在设置中随时切换）：",
    normal: "普通用户",
    normalDesc: ["图形化安装向导", "自动沙盒隔离（安全）", "一键升级与安全提醒", "无需技术知识"],
    dev: "开发者",
    devDesc: ["完整 CLI 工具链", "多实例管理", "Native / 沙盒模式自选", "Skill 开发脚手架", "快照与回滚"],
  },
  en: {
    title: "ClawEnv",
    subtitle: "OpenClaw Installer & Manager",
    desc: "ClawEnv provides cross-platform secure sandbox installation, upgrade and management for the OpenClaw AI Agent framework. Supports macOS (Lima), Windows (WSL2), Linux (Podman).",
    choose: "Choose your usage mode (can be changed in Settings):",
    normal: "Normal User",
    normalDesc: ["Guided installation wizard", "Automatic sandbox (secure)", "One-click upgrade & alerts", "No technical knowledge required"],
    dev: "Developer",
    devDesc: ["Full CLI toolchain", "Multi-instance management", "Native or sandbox mode", "Skill development scaffold", "Snapshot & rollback"],
  },
};

type Lang = keyof typeof i18n;

export default function ModeSelect(props: { onComplete: () => void }) {
  const [lang, setLang] = createSignal<Lang>("zh-CN");
  const t = () => i18n[lang()];

  async function selectMode(mode: "general" | "developer") {
    try { await invoke("create_default_config", { userMode: mode }); } catch {}
    props.onComplete();
  }

  return (
    <div class="flex h-screen items-center justify-center bg-gray-900 text-white relative">
      {/* Language switcher — top right */}
      <div class="absolute top-4 right-5 flex gap-1">
        <button
          class={`px-2 py-0.5 text-xs rounded ${lang() === "zh-CN" ? "bg-indigo-600" : "bg-gray-700 hover:bg-gray-600"}`}
          onClick={() => setLang("zh-CN")}
        >
          中文
        </button>
        <button
          class={`px-2 py-0.5 text-xs rounded ${lang() === "en" ? "bg-indigo-600" : "bg-gray-700 hover:bg-gray-600"}`}
          onClick={() => setLang("en")}
        >
          EN
        </button>
      </div>

      <div class="text-center max-w-2xl px-8">
        <h1 class="text-3xl font-bold mb-1">{t().title}</h1>
        <p class="text-gray-400 mb-6 text-sm">{t().subtitle}</p>

        <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 mb-8 text-sm text-gray-300 text-left">
          {t().desc}
        </div>

        <p class="text-sm text-gray-400 mb-5">{t().choose}</p>

        <div class="grid grid-cols-2 gap-5">
          <button
            class="bg-gray-800 border border-gray-700 rounded-xl p-5 text-left hover:border-indigo-500 transition-colors group"
            onClick={() => selectMode("general")}
          >
            <div class="text-base font-medium mb-2 group-hover:text-indigo-400">{t().normal}</div>
            <ul class="text-sm text-gray-400 space-y-1">
              {t().normalDesc.map((d) => <li>- {d}</li>)}
            </ul>
          </button>

          <button
            class="bg-gray-800 border border-gray-700 rounded-xl p-5 text-left hover:border-indigo-500 transition-colors group"
            onClick={() => selectMode("developer")}
          >
            <div class="text-base font-medium mb-2 group-hover:text-indigo-400">{t().dev}</div>
            <ul class="text-sm text-gray-400 space-y-1">
              {t().devDesc.map((d) => <li>- {d}</li>)}
            </ul>
          </button>
        </div>
      </div>
    </div>
  );
}

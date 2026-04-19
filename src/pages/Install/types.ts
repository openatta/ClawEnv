export type InstallProgress = { message: string; percent: number; stage: string };
export type ConnTestResult = { endpoint: string; ok: boolean; message: string };
export type SystemProxy = { detected: boolean; source: string; http_proxy: string; https_proxy: string; no_proxy: string };
export type CheckItem = { name: string; ok: boolean; detail: string; info_only?: boolean };
export type SystemCheckInfo = { os: string; arch: string; memory_gb: number; disk_free_gb: number; sandbox_backend: string; sandbox_available: boolean; checks: CheckItem[] };

export type InstallState = {
  instanceName: string;
  clawType: string;
  clawDisplayName: string;
  installMethod: "online" | "local" | "native" | "native-import";
  localFilePath: string;
  apiKey: string;
  installBrowser: boolean;
  installMcpBridge: boolean;
  /** Serialized ProxyConfig JSON from StepNetwork, or null = no proxy chosen.
   *  Passed to the install IPC so HTTPS_PROXY etc. get injected into the
   *  clawcli child process for this install only (not persisted). */
  proxyJson: string | null;
};

export function makeInstallStages(name: string) {
  return [
    { key: "detect_platform", label: "检测平台 / Detect Platform" },
    { key: "ensure_prerequisites", label: "检查前置条件 / Prerequisites" },
    { key: "create_vm", label: "创建虚拟机 / Create VM" },
    { key: "boot_vm", label: "启动虚拟机 / Boot VM" },
    { key: "configure_proxy", label: "配置代理 / Configure Proxy" },
    { key: "install_deps", label: "安装依赖 / Install Dependencies" },
    { key: "install_open_claw", label: `安装 ${name} / Install ${name}` },
    { key: "store_api_key", label: "存储 API Key / Store API Key" },
    { key: "install_browser", label: "安装浏览器 / Install Browser" },
    { key: "start_open_claw", label: `启动 ${name} / Start ${name}` },
    { key: "save_config", label: "保存配置 / Save Config" },
  ];
}

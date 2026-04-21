export default function StepWelcome(props: {
  lang: "zh-CN" | "en";
  instanceName: string;
  onInstanceNameChange: (name: string) => void;
  nameError: string;
  clawDisplayName: string;
  onLangChange: (lang: "zh-CN" | "en") => void;
}) {
  const zh = () => props.lang === "zh-CN";
  return (
    <div>
      <div class="flex items-center justify-between mb-3">
        <h2 class="text-xl font-bold">{zh() ? "欢迎使用 ClawEnv" : "Welcome to ClawEnv"}</h2>
        <div class="flex gap-1">
          <button class={`px-2 py-0.5 text-xs rounded ${zh() ? "bg-indigo-600" : "bg-gray-700"}`} onClick={() => props.onLangChange("zh-CN")}>中文</button>
          <button class={`px-2 py-0.5 text-xs rounded ${!zh() ? "bg-indigo-600" : "bg-gray-700"}`} onClick={() => props.onLangChange("en")}>EN</button>
        </div>
      </div>
      <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 mb-4 text-sm text-gray-300 space-y-2">
        {zh() ? (<>
          <p><strong>ClawEnv</strong> 是 <strong>{props.clawDisplayName}</strong> 的跨平台沙盒安装器与管理工具。</p>
          <p>它在您的系统上创建安全隔离的沙盒环境（Alpine Linux），让 {props.clawDisplayName} 安全运行而不影响宿主系统。</p>
          <p class="text-gray-400">安装向导将：</p>
          <ul class="list-disc list-inside text-gray-400 space-y-1">
            <li>检查系统是否满足要求（操作系统、内存、磁盘空间）</li>
            <li>配置网络和代理设置</li>
            <li>下载并在沙盒中安装 {props.clawDisplayName}</li>
            <li>安装完成后，{props.clawDisplayName} 自己的管理界面会收集 API Key（ClawEnv 不经手凭证）</li>
          </ul>
          <p class="text-gray-500 text-xs mt-2">支持平台：macOS (Lima)、Windows (WSL2)、Linux (Podman)</p>
        </>) : (<>
          <p><strong>ClawEnv</strong> is a cross-platform sandbox installer and manager for <strong>{props.clawDisplayName}</strong>.</p>
          <p>It creates a secure, isolated sandbox environment (Alpine Linux) on your system, so {props.clawDisplayName} runs safely without affecting your host OS.</p>
          <p class="text-gray-400">This wizard will:</p>
          <ul class="list-disc list-inside text-gray-400 space-y-1">
            <li>Check your system meets requirements (OS, memory, disk)</li>
            <li>Configure network & proxy settings</li>
            <li>Download and install {props.clawDisplayName} in a sandbox</li>
            <li>Post-install, {props.clawDisplayName}'s own management UI collects your API key — ClawEnv never handles credentials</li>
          </ul>
          <p class="text-gray-500 text-xs mt-2">Supported: macOS (Lima), Windows (WSL2), Linux (Podman)</p>
        </>)}
      </div>

      {/* Instance name */}
      <div class="mt-4">
        <label class="block text-sm text-gray-400 mb-1">
          {zh() ? "实例名称" : "Instance Name"}
        </label>
        <input
          type="text"
          value={props.instanceName}
          onInput={(e) => props.onInstanceNameChange(e.currentTarget.value.replace(/[^a-zA-Z0-9_-]/g, ""))}
          placeholder="default"
          class={`bg-gray-800 border rounded px-3 py-2 w-64 text-sm ${props.nameError ? "border-red-500" : "border-gray-600"}`}
        />
        {props.nameError ? (
          <p class="text-xs text-red-400 mt-1">{props.nameError}</p>
        ) : (
          <p class="text-xs text-gray-500 mt-1">
            {zh() ? "字母、数字、连字符、下划线，用于区分多个实例" : "Letters, numbers, hyphens, underscores. Used to identify this instance."}
          </p>
        )}
      </div>
    </div>
  );
}

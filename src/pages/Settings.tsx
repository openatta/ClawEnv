import { createSignal, Show, For, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type BridgeConfig = {
  enabled: boolean;
  port: number;
  permissions: {
    file_read: string[];
    file_write: string[];
    file_deny: string[];
    exec_allow: string[];
    exec_deny: string[];
    require_approval: string[];
    auto_approve: string[];
    shell_enabled: boolean;
    shell_program: string;
    shell_require_approval: boolean;
  };
};

export default function Settings() {
  const [tab, setTab] = createSignal<"general" | "bridge">("general");
  const [saving, setSaving] = createSignal(false);
  const [saveMsg, setSaveMsg] = createSignal("");

  // General settings
  const [language, setLanguage] = createSignal("zh-CN");
  const [theme, setTheme] = createSignal("system");
  const [trayEnabled, setTrayEnabled] = createSignal(true);
  const [startMinimized, setStartMinimized] = createSignal(false);
  const [showNotifications, setShowNotifications] = createSignal(true);
  const [proxyEnabled, setProxyEnabled] = createSignal(false);
  const [httpProxy, setHttpProxy] = createSignal("");
  const [noProxy, setNoProxy] = createSignal("localhost,127.0.0.1");
  const [autoCheck, setAutoCheck] = createSignal(true);

  // Bridge settings
  const [bridgeEnabled, setBridgeEnabled] = createSignal(false);
  const [bridgePort, setBridgePort] = createSignal(3100);
  const [fileRead, setFileRead] = createSignal("");
  const [fileWrite, setFileWrite] = createSignal("");
  const [fileDeny, setFileDeny] = createSignal("");
  const [execAllow, setExecAllow] = createSignal("");
  const [execDeny, setExecDeny] = createSignal("");
  const [shellEnabled, setShellEnabled] = createSignal(false);
  const [shellProgram, setShellProgram] = createSignal("bash");
  const [shellApproval, setShellApproval] = createSignal(true);

  onMount(async () => {
    try {
      const cfg = await invoke<BridgeConfig>("get_bridge_config");
      setBridgeEnabled(cfg.enabled);
      setBridgePort(cfg.port);
      setFileRead(cfg.permissions.file_read.join(", "));
      setFileWrite(cfg.permissions.file_write.join(", "));
      setFileDeny(cfg.permissions.file_deny.join(", "));
      setExecAllow(cfg.permissions.exec_allow.join(", "));
      setExecDeny(cfg.permissions.exec_deny.join(", "));
      setShellEnabled(cfg.permissions.shell_enabled);
      setShellProgram(cfg.permissions.shell_program);
      setShellApproval(cfg.permissions.shell_require_approval);
    } catch {}
  });

  function showSaved(msg = "Saved") {
    setSaveMsg(msg);
    setTimeout(() => setSaveMsg(""), 2000);
  }

  async function saveGeneral() {
    setSaving(true);
    try {
      await invoke("save_settings", {
        settingsJson: JSON.stringify({
          language: language(), theme: theme(),
          auto_check_updates: autoCheck(),
          proxy: { enabled: proxyEnabled(), http_proxy: httpProxy(), https_proxy: "", no_proxy: noProxy(), auth_required: false, auth_user: "" },
        }),
      });
      showSaved();
    } catch (e) { showSaved("Error: " + e); }
    finally { setSaving(false); }
  }

  async function saveBridge() {
    setSaving(true);
    const split = (s: string) => s.split(",").map(x => x.trim()).filter(x => x);
    try {
      await invoke("save_bridge_config", {
        bridgeJson: JSON.stringify({
          enabled: bridgeEnabled(),
          port: bridgePort(),
          permissions: {
            file_read: split(fileRead()),
            file_write: split(fileWrite()),
            file_deny: split(fileDeny()),
            exec_allow: split(execAllow()),
            exec_deny: split(execDeny()),
            require_approval: ["file_write", "exec"],
            auto_approve: ["file_read"],
            shell_enabled: shellEnabled(),
            shell_program: shellProgram(),
            shell_require_approval: shellApproval(),
          },
        }),
      });
      showSaved();
    } catch (e) { showSaved("Error: " + e); }
    finally { setSaving(false); }
  }

  return (
    <div class="h-full flex flex-col">
      {/* Header */}
      <div class="flex items-center justify-between px-6 py-4 border-b border-gray-800 shrink-0">
        <h1 class="text-xl font-bold">Settings</h1>
        <div class="flex items-center gap-3">
          {saveMsg() && (
            <span class={`text-sm ${saveMsg().startsWith("Error") ? "text-red-400" : "text-green-400"}`}>
              {saveMsg()}
            </span>
          )}
          <button
            class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50"
            disabled={saving()}
            onClick={() => tab() === "bridge" ? saveBridge() : saveGeneral()}
          >
            {saving() ? "Saving..." : "Save"}
          </button>
        </div>
      </div>

      {/* Tabs */}
      <div class="flex border-b border-gray-800 px-4 shrink-0">
        <button class={`px-4 py-2 text-sm border-b-2 transition-colors ${tab() === "general" ? "border-indigo-500 text-white" : "border-transparent text-gray-400"}`}
          onClick={() => setTab("general")}>General</button>
        <button class={`px-4 py-2 text-sm border-b-2 transition-colors ${tab() === "bridge" ? "border-indigo-500 text-white" : "border-transparent text-gray-400"}`}
          onClick={() => setTab("bridge")}>Bridge Server</button>
      </div>

      {/* Content */}
      <div class="flex-1 overflow-y-auto p-6">

        {/* ===== General Tab ===== */}
        <Show when={tab() === "general"}>
          <section class="mb-8">
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">General</h2>
            <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
              <Row label="Language">
                <select class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600" value={language()} onChange={e => setLanguage(e.target.value)}>
                  <option value="zh-CN">简体中文</option>
                  <option value="en">English</option>
                </select>
              </Row>
              <Row label="Theme">
                <select class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600" value={theme()} onChange={e => setTheme(e.target.value)}>
                  <option value="system">System</option>
                  <option value="dark">Dark</option>
                  <option value="light">Light</option>
                </select>
              </Row>
            </div>
          </section>

          <section class="mb-8">
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">System Tray</h2>
            <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
              <Row label="Enable tray"><Toggle checked={trayEnabled()} onChange={setTrayEnabled} /></Row>
              <Row label="Start minimized"><Toggle checked={startMinimized()} onChange={setStartMinimized} /></Row>
              <Row label="Show notifications"><Toggle checked={showNotifications()} onChange={setShowNotifications} /></Row>
            </div>
          </section>

          <section class="mb-8">
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Network</h2>
            <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
              <Row label="Use proxy"><Toggle checked={proxyEnabled()} onChange={setProxyEnabled} /></Row>
              <Row label="HTTP Proxy">
                <input type="text" placeholder="http://proxy:8080" class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600 w-64" value={httpProxy()} onInput={e => setHttpProxy(e.target.value)} />
              </Row>
              <Row label="No proxy">
                <input type="text" class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600 w-64" value={noProxy()} onInput={e => setNoProxy(e.target.value)} />
              </Row>
            </div>
          </section>

          <section class="mb-8">
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Updates</h2>
            <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
              <Row label="Auto check updates"><Toggle checked={autoCheck()} onChange={setAutoCheck} /></Row>
            </div>
          </section>

          <section>
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">About</h2>
            <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 text-sm text-gray-400">
              <p>ClawEnv v0.2.0</p>
              <p>OpenClaw sandbox installer & manager</p>
            </div>
          </section>
        </Show>

        {/* ===== Bridge Tab ===== */}
        <Show when={tab() === "bridge"}>
          <section class="mb-6">
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Bridge Server</h2>
            <p class="text-xs text-gray-500 mb-4">
              The Bridge Server runs on the host and provides a controlled HTTP API for sandbox agents
              to access host files and commands. All access is governed by permission rules below.
            </p>
            <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
              <Row label="Enable Bridge Server">
                <Toggle checked={bridgeEnabled()} onChange={setBridgeEnabled} />
              </Row>
              <Row label="Port">
                <input type="number" class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600 w-24"
                  value={bridgePort()} onInput={e => setBridgePort(parseInt(e.target.value) || 3100)} />
              </Row>
              <Show when={bridgeEnabled()}>
                <div class="text-xs text-green-400 flex items-center gap-1.5">
                  <span>●</span> Endpoint: http://127.0.0.1:{bridgePort()}/api/health
                </div>
              </Show>
            </div>
          </section>

          <section class="mb-6">
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">File Permissions</h2>
            <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
              <PermRow label="File Read (allow)" hint="Glob patterns, comma-separated" value={fileRead()} onChange={setFileRead} />
              <PermRow label="File Write (allow)" hint="Glob patterns" value={fileWrite()} onChange={setFileWrite} />
              <PermRow label="File Deny (block)" hint="These override allow rules" value={fileDeny()} onChange={setFileDeny} color="red" />
            </div>
          </section>

          <section class="mb-6">
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Command Permissions</h2>
            <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
              <PermRow label="Exec Allow" hint="Programs agents can call" value={execAllow()} onChange={setExecAllow} />
              <PermRow label="Exec Deny (block)" hint="Blocked commands" value={execDeny()} onChange={setExecDeny} color="red" />
            </div>
          </section>

          <section class="mb-6">
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Shell Access</h2>
            <p class="text-xs text-gray-500 mb-3">
              Shell mode allows agents to execute arbitrary scripts on the host.
              This is powerful but dangerous — enable with caution.
            </p>
            <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-yellow-700/30">
              <Row label="Enable Shell Mode">
                <Toggle checked={shellEnabled()} onChange={setShellEnabled} />
              </Row>
              <Show when={shellEnabled()}>
                <Row label="Shell Program">
                  <select class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600" value={shellProgram()} onChange={e => setShellProgram(e.target.value)}>
                    <option value="bash">bash</option>
                    <option value="sh">sh</option>
                    <option value="zsh">zsh</option>
                    <option value="powershell">PowerShell</option>
                  </select>
                </Row>
                <Row label="Require approval for each command">
                  <Toggle checked={shellApproval()} onChange={setShellApproval} />
                </Row>
                <Show when={!shellApproval()}>
                  <div class="text-xs text-red-400 flex items-center gap-1.5">
                    ⚠ WARNING: Shell commands will execute WITHOUT user confirmation.
                    This is a significant security risk.
                  </div>
                </Show>
              </Show>
            </div>
          </section>

          <section>
            <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Approval Policy</h2>
            <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 text-sm text-gray-400 space-y-1">
              <div>☑ <span class="text-gray-300">file_write</span> — requires user approval</div>
              <div>☑ <span class="text-gray-300">exec</span> — requires user approval</div>
              <div>☐ <span class="text-gray-300">file_read</span> — auto-approved</div>
            </div>
          </section>
        </Show>
      </div>
    </div>
  );
}

function Row(props: { label: string; children: any }) {
  return (
    <div class="flex items-center justify-between">
      <span class="text-sm">{props.label}</span>
      {props.children}
    </div>
  );
}

function PermRow(props: { label: string; hint: string; value: string; onChange: (v: string) => void; color?: string }) {
  return (
    <div>
      <div class="flex items-center justify-between mb-1">
        <span class={`text-sm ${props.color === "red" ? "text-red-400" : ""}`}>{props.label}</span>
        <span class="text-[10px] text-gray-500">{props.hint}</span>
      </div>
      <input type="text" class={`w-full bg-gray-700 text-sm rounded px-2 py-1.5 border ${props.color === "red" ? "border-red-700/50" : "border-gray-600"}`}
        value={props.value} onInput={e => props.onChange(e.target.value)} />
    </div>
  );
}

function Toggle(props: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button class={`w-10 h-5 rounded-full transition-colors ${props.checked ? "bg-indigo-600" : "bg-gray-600"}`}
      onClick={() => props.onChange(!props.checked)}>
      <div class={`w-4 h-4 bg-white rounded-full transform transition-transform ${props.checked ? "translate-x-5" : "translate-x-0.5"}`} />
    </button>
  );
}

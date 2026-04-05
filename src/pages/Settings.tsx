import { createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

export default function Settings() {
  const [language, setLanguage] = createSignal("zh-CN");
  const [theme, setTheme] = createSignal("system");
  const [trayEnabled, setTrayEnabled] = createSignal(true);
  const [startMinimized, setStartMinimized] = createSignal(false);
  const [showNotifications, setShowNotifications] = createSignal(true);
  const [proxyEnabled, setProxyEnabled] = createSignal(false);
  const [httpProxy, setHttpProxy] = createSignal("");
  const [noProxy, setNoProxy] = createSignal("localhost,127.0.0.1");
  const [autoCheck, setAutoCheck] = createSignal(true);
  const [autoSnapshot, setAutoSnapshot] = createSignal(true);
  const [saving, setSaving] = createSignal(false);
  const [saveMsg, setSaveMsg] = createSignal("");

  async function save() {
    setSaving(true);
    setSaveMsg("");
    try {
      await invoke("save_settings", {
        settingsJson: JSON.stringify({
          language: language(),
          theme: theme(),
          auto_check_updates: autoCheck(),
          proxy: {
            enabled: proxyEnabled(),
            http_proxy: httpProxy(),
            https_proxy: "",
            no_proxy: noProxy(),
            auth_required: false,
            auth_user: "",
          },
        }),
      });
      setSaveMsg("Saved");
      setTimeout(() => setSaveMsg(""), 2000);
    } catch (e) {
      setSaveMsg("Error: " + String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div class="h-full overflow-y-auto p-6">
      <div class="flex items-center justify-between mb-6">
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
            onClick={save}
          >
            {saving() ? "Saving..." : "Save"}
          </button>
        </div>
      </div>

      <section class="mb-8">
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">General</h2>
        <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
          <Row label="Language">
            <select
              class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600"
              value={language()}
              onChange={(e) => setLanguage(e.target.value)}
            >
              <option value="zh-CN">简体中文</option>
              <option value="en">English</option>
            </select>
          </Row>
          <Row label="Theme">
            <select
              class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600"
              value={theme()}
              onChange={(e) => setTheme(e.target.value)}
            >
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
          <Row label="Enable tray">
            <Toggle checked={trayEnabled()} onChange={setTrayEnabled} />
          </Row>
          <Row label="Start minimized">
            <Toggle checked={startMinimized()} onChange={setStartMinimized} />
          </Row>
          <Row label="Show notifications">
            <Toggle checked={showNotifications()} onChange={setShowNotifications} />
          </Row>
        </div>
      </section>

      <section class="mb-8">
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Network</h2>
        <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
          <Row label="Use proxy">
            <Toggle checked={proxyEnabled()} onChange={setProxyEnabled} />
          </Row>
          <Row label="HTTP Proxy">
            <input
              type="text"
              placeholder="http://proxy:8080"
              class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600 w-64"
              value={httpProxy()}
              onInput={(e) => setHttpProxy(e.target.value)}
            />
          </Row>
          <Row label="No proxy">
            <input
              type="text"
              class="bg-gray-700 text-sm rounded px-2 py-1 border border-gray-600 w-64"
              value={noProxy()}
              onInput={(e) => setNoProxy(e.target.value)}
            />
          </Row>
        </div>
      </section>

      <section class="mb-8">
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">Updates</h2>
        <div class="space-y-4 bg-gray-800 rounded-lg p-4 border border-gray-700">
          <Row label="Auto check updates">
            <Toggle checked={autoCheck()} onChange={setAutoCheck} />
          </Row>
          <Row label="Auto snapshot before upgrade">
            <Toggle checked={autoSnapshot()} onChange={setAutoSnapshot} />
          </Row>
        </div>
      </section>

      <section>
        <h2 class="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">About</h2>
        <div class="bg-gray-800 rounded-lg p-4 border border-gray-700 text-sm text-gray-400">
          <p>ClawEnv v0.1.0</p>
          <p>OpenClaw sandbox installer & manager</p>
        </div>
      </section>
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

function Toggle(props: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      class={`w-10 h-5 rounded-full transition-colors ${props.checked ? "bg-indigo-600" : "bg-gray-600"}`}
      onClick={() => props.onChange(!props.checked)}
    >
      <div
        class={`w-4 h-4 bg-white rounded-full transform transition-transform ${
          props.checked ? "translate-x-5" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}

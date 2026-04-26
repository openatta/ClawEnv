import { createSignal, onMount, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../../../i18n";

/** Shape returned by `lite_scan_packages` / `lite_inspect_package` IPC
 *  — mirrors `tauri/src/ipc/lite.rs::PackageInfo`. Kept inline rather
 *  than imported from a shared types module because lite is the only
 *  consumer.
 *
 *  `claw_type` / `claw_version` / `claw_display_name` come from the
 *  bundle's own `clawenv-bundle.toml` manifest — authoritative over
 *  the filename. An empty `claw_type` means the manifest was missing
 *  or unreadable; such a bundle is always marked `compatible=false`
 *  with a `reason` string the UI surfaces.
 */
export type PackageInfo = {
  path: string;
  filename: string;
  platform: string;
  arch: string;
  is_native: boolean;
  size_mb: number;
  compatible: boolean;
  needs_sandbox_backend: boolean;
  sandbox_backend_available: boolean;
  claw_type: string;
  claw_version: string;
  claw_display_name: string;
  reason: string;
};

/** Lite-only step: scan the install folder for `*.tar.gz` bundles, show
 *  compatibility state, block the user from picking a native bundle when
 *  the host already has a native instance installed. Includes a "Choose
 *  file..." escape hatch for bundles outside the app folder.
 *
 *  "Incompatible-with-current-OS/arch" bundles stay visible but greyed
 *  out & disabled — matches user intent of "let the user see what's
 *  there, but only allow the matching pick".
 *
 *  `native conflict` is separate from `compatible`: a native-for-this-
 *  arch bundle IS compatible in the OS/arch sense, but if the user
 *  already has a native instance we reject it (since v0.2.x's
 *  architecture铁律 allows only one native instance). Red warning, not
 *  a grey-out, so the user sees WHY they can't pick it.
 */
export default function LiteStepScan(props: {
  /** Optional: when set, only bundles whose manifest claw_type equals
   *  this value are shown. Used by LiteInstallFlow when the user hit
   *  "+" on a specific claw tab in Home / ClawPage — otherwise they
   *  could accidentally install an OpenClaw bundle while trying to
   *  add a Hermes instance. Undefined = no filter (show everything
   *  compatible). Case-insensitive match on manifest claw_type. */
  filterClawType?: string;
  onPick: (pkg: PackageInfo) => void;
  onBack: () => void;
}) {
  const [packages, setPackages] = createSignal<PackageInfo[]>([]);
  const [selected, setSelected] = createSignal<PackageInfo | null>(null);
  const [scanning, setScanning] = createSignal(true);
  const [hasNative, setHasNative] = createSignal(false);
  const [pickError, setPickError] = createSignal("");
  // Current scan directory — null = the app's own folder (default).
  // Set via the "Choose folder..." button so the user can point Lite at
  // bundles outside the app folder (USB, network drive, Downloads).
  const [scanDir, setScanDir] = createSignal<string | null>(null);

  /** Apply the claw-type filter AFTER scanning so the scanner's own
   *  "No .tar.gz bundles found" branch still triggers on an empty
   *  folder. If we filtered inside the IPC we couldn't distinguish
   *  "folder empty" from "folder has bundles of other types". */
  const filtered = () => {
    const want = props.filterClawType?.toLowerCase();
    if (!want) return packages();
    return packages().filter(p => p.claw_type.toLowerCase() === want);
  };

  /** Run a scan against the given directory (null = app folder) and
   *  update the list + auto-select the first installable bundle. Shared
   *  between the initial onMount scan and the "Choose folder..." flow
   *  so both code paths go through identical compatibility/filter
   *  logic — the user's view never diverges from what the backend
   *  reported. */
  async function runScan(dir: string | null) {
    setScanning(true);
    setPickError("");
    const pkgs = await invoke<PackageInfo[]>("lite_scan_packages", { scanDir: dir }).catch(() => []);
    setPackages(pkgs);

    const want = props.filterClawType?.toLowerCase();
    const nativeExists = hasNative();
    const installable = pkgs.find(p =>
      p.compatible
      && !(p.is_native && nativeExists)
      && (!want || p.claw_type.toLowerCase() === want)
    );
    setSelected(installable ?? null);
    setScanning(false);
  }

  onMount(async () => {
    // Native-instance check up-front — static over the life of this step,
    // so we only fetch once. hasNative() feeds into every scan's
    // auto-select + the red-warning gate in the list below.
    const nativeExists = await invoke<boolean>("has_native_instance").catch(() => false);
    setHasNative(nativeExists);
    await runScan(null);
  });

  const isBlocked = (pkg: PackageInfo) =>
    !pkg.compatible || (pkg.is_native && hasNative());

  /** Escape hatch: open native folder picker, re-scan against the chosen
   *  directory. Keeps the behaviour symmetric with the default app-folder
   *  scan — list all compatible bundles, grey-out incompatible ones, let
   *  the user pick. */
  async function chooseFolder() {
    setPickError("");
    try {
      const picked = await invoke<string | null>("pick_import_folder");
      if (!picked) return; // user cancelled
      setScanDir(picked);
      await runScan(picked);
    } catch (e) {
      setPickError(String(e));
    }
  }

  return (
    <div class="flex flex-col h-full">
      <h2 class="text-xl font-bold mb-2">
        {t("选择安装包", "Select Package")}
      </h2>
      <p class="text-sm text-gray-400 mb-1">
        {scanDir()
          ? t("已扫描你选择的目录。选择一个与本机匹配的安装包继续。",
               "Scanned the folder you chose. Pick a bundle that matches your machine.")
          : t("已扫描程序所在目录。选择一个与本机匹配的安装包继续。",
               "Scanned the folder this app lives in. Pick a bundle that matches your machine.")}
      </p>
      <p class="text-xs text-gray-500 font-mono mb-4 truncate" title={scanDir() || ""}>
        {scanDir() ? `📁 ${scanDir()}` : ""}
      </p>

      <div class="flex-1 overflow-y-auto">
        <Show when={scanning()}>
          <p class="text-sm text-gray-400 animate-pulse">{t("扫描中...", "Scanning...")}</p>
        </Show>

        <Show when={!scanning() && packages().length === 0}>
          <div class="text-sm text-red-400 bg-red-900/20 border border-red-700/50 rounded p-3">
            {t("此目录下没有 .tar.gz 安装包。请将 ClawEnv 为你准备的安装包文件放到此目录，或使用下方「选择目录...」按钮选择其他目录。",
               "No .tar.gz bundles in this folder. Place the bundle ClawEnv prepared for you here, or use the \"Choose folder...\" button below to pick a different folder.")}
          </div>
        </Show>

        {/* Filter active + no matching bundle: specific message that
            names the expected claw type. Distinct from the empty-folder
            case above. */}
        <Show when={!scanning() && packages().length > 0 && filtered().length === 0 && props.filterClawType}>
          <div class="text-sm text-red-400 bg-red-900/20 border border-red-700/50 rounded p-3">
            {t(
              `此目录下没有 ${props.filterClawType} 类型的安装包。使用下方「选择目录...」按钮选择其他目录。`,
              `No ${props.filterClawType} bundles in this folder. Use the "Choose folder..." button below to pick a different folder.`
            )}
          </div>
        </Show>

        <Show when={!scanning() && filtered().length > 0}>
          <div class="space-y-2 mb-4">
            <For each={filtered()}>
              {(pkg) => {
                const blocked = () => isBlocked(pkg);
                const nativeConflict = () => pkg.is_native && hasNative() && pkg.compatible;
                // Prefer the claw display name + version from the manifest;
                // fall back to the filename when the manifest was unreadable
                // (which the `reason` field will also flag).
                const titleLine = () => pkg.claw_display_name
                  ? `${pkg.claw_display_name}${pkg.claw_version ? " " + pkg.claw_version : ""}`
                  : pkg.filename;
                return (
                  <label class={`flex items-start gap-3 p-3 rounded border transition-colors ${
                    blocked() ? "opacity-60 cursor-not-allowed border-gray-700 bg-gray-800/40" :
                    selected() === pkg ? "border-indigo-500 bg-indigo-900/20 cursor-pointer" :
                    "border-gray-700 hover:border-gray-500 cursor-pointer"
                  }`}>
                    <input type="radio" name="pkg" disabled={blocked()}
                      checked={selected() === pkg}
                      onChange={() => !blocked() && setSelected(pkg)}
                      class="mt-1 w-4 h-4 shrink-0" />
                    <div class="flex-1 min-w-0">
                      <div class="text-sm font-medium truncate">{titleLine()}</div>
                      <div class="text-xs text-gray-500 font-mono truncate">{pkg.filename}</div>
                      <div class="text-xs text-gray-400 mt-0.5">
                        {pkg.is_native
                          ? t("本地安装 (Native)", "Native Bundle")
                          : t("沙盒镜像 (Sandbox)", "Sandbox Image")}
                        {" · "}{pkg.size_mb} MB
                        {pkg.platform && pkg.arch
                          ? ` · ${pkg.platform}/${pkg.arch}`
                          : ""}
                      </div>

                      {/* Stacked status lines — each independent so more
                          than one may appear (e.g. incompatible + also
                          needs a sandbox backend). Order: hardest block
                          first. */}
                      <Show when={!pkg.compatible && pkg.reason}>
                        <div class="text-xs text-red-400 mt-1">
                          ✗ {pkg.reason}
                        </div>
                      </Show>
                      <Show when={nativeConflict()}>
                        <div class="text-xs text-red-400 mt-1 font-medium">
                          {t("✗ 本地已有 Native 安装，不能再装一个",
                             "✗ A native instance already exists — cannot install another")}
                        </div>
                      </Show>
                      <Show when={pkg.needs_sandbox_backend && !pkg.sandbox_backend_available && pkg.compatible}>
                        <div class="text-xs text-yellow-400 mt-1">
                          {pkg.platform === "lima"
                            ? t("⚠ 将首先安装 Lima（无需重启）",
                                "⚠ Lima will be installed first (no restart needed)")
                            : t("⚠ 将首先安装 WSL2（可能需要重启 Windows）",
                                "⚠ WSL2 will be installed first (Windows restart may be required)")}
                        </div>
                      </Show>
                    </div>
                  </label>
                );
              }}
            </For>
          </div>
        </Show>

        <div class="flex items-center gap-2 mt-2">
          <button class="px-3 py-1.5 text-xs bg-gray-800 hover:bg-gray-700 rounded border border-gray-700"
            onClick={chooseFolder}>
            {t("选择目录...", "Choose folder...")}
          </button>
          <Show when={pickError()}>
            <span class="text-xs text-red-400">{pickError()}</span>
          </Show>
        </div>
      </div>

      {/* Nav */}
      <div class="flex justify-between pt-3 border-t border-gray-800 shrink-0">
        <button class="px-4 py-1.5 text-sm bg-gray-800 hover:bg-gray-700 rounded"
          onClick={props.onBack}>
          {t("上一步", "Back")}
        </button>
        <button class="px-4 py-1.5 text-sm bg-indigo-600 hover:bg-indigo-500 rounded disabled:opacity-50 disabled:cursor-not-allowed"
          disabled={!selected() || isBlocked(selected()!)}
          onClick={() => { const p = selected(); if (p) props.onPick(p); }}>
          {t("下一步", "Next")}
        </button>
      </div>
    </div>
  );
}

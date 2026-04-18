# Bundle 格式规范 (v0.2.6+)

> SSOT：这份文档和 `core/src/export/manifest.rs` 是同一份契约的两种投影。
> 修改任何一份都要同步另一份，否则导出方与导入方的假设会漂移。

ClawEnv 的 import/export 产出 / 消费的 `.tar.gz` 包统称 **Bundle**。从 v0.2.6 起，每个 Bundle **必须**在归档根部携带一份 `clawenv-bundle.toml` manifest；没有 manifest 的包被 **拒绝导入**（用户决定，无兼容 shim）。

## 为什么要 manifest

在 v0.2.5 之前，导入一侧靠 "依次跑每个 claw 的 `version_check_cmd` 看哪个返回非空" 来推断包里装的是 OpenClaw 还是 Hermes。这种启发式有两个硬伤：

1. **脆**：某个 claw 的 version 命令偶然打印了东西就会误判；共享 binary 的 claw 冲突。
2. **慢**：每次导入都要把整个 tar 摊开 + 启动沙箱才能开始探测。

Manifest 把 claw 身份和 sandbox 后端做成了 **发送方权威声明**，导入方 peek 即可决策，解包之前就能 fail fast。

## Manifest schema

```toml
schema_version    = 1                              # u32, 只有 reader 能识别才放行
clawenv_version   = "0.2.6"                        # 产生此包的 clawenv 发布版本
created_at        = "2026-04-18T15:00:00+00:00"    # RFC-3339 UTC
claw_type         = "hermes"                       # registry 里的 claw id ("openclaw"/"hermes"/...)
claw_version      = "Hermes Agent v0.10.0 (2026.4.16)"  # 可包含多行，仅供展示
sandbox_type      = "lima-alpine"                  # SandboxType 的 kebab-case wire 形式
source_platform   = "macos-aarch64"                # 产生方 OS-arch，informational
```

- `schema_version` — 单调递增；导入端见到 > 自己支持的 max 就 bail 并提示升级 clawenv。
- `sandbox_type` — 严格匹配 `SandboxType::as_wire_str()`：`lima-alpine` / `wsl2-alpine` / `podman-alpine` / `native`。import 侧会和本机的 `SandboxType::from_os()` 比对，跨 backend 导入直接拒绝。
- `claw_version` — 拿源端运行时 `version_check_cmd` 的完整 stdout（常含换行），纯展示用，不解析。

## 归档物理结构

四个 backend 的打包产物在**外层**是统一的 `.tar.gz`，**内部结构**分两类：

### A. Native / Lima — manifest + 文件树并排

```
bundle.tar.gz (gzip)
└── <tar archive>
    ├── clawenv-bundle.toml       ← manifest 必在根部
    ├── node/                     (Native) 私有 Node.js
    ├── git/                      (Native) 私有 Git
    ├── native/                   (Native) claw 安装
    └── <vm_name>/                (Lima)   lima VM dir: lima.yaml / basedisk / cidata.iso / ...
```

Lima 排除了运行时垃圾：`*.sock` / `*.pid` / `*.log`。`cidata.iso` **保留**（新机器首次 boot 要 cloud-init seed）。

### B. Podman / WSL — manifest + wrapped inner payload

`podman save` / `wsl --export` 产出的 tar 是 **container image tar** / **distro tar**，不是普通文件系统 tar，**不能**跟 manifest 混在同一个归档里。所以采用"外层 wrap"：

```
bundle.tar.gz
└── <tar archive>
    ├── clawenv-bundle.toml       ← manifest 必在根部
    └── payload.tar               ← 原样 podman save / wsl --export 输出
```

`payload.tar` 的名字是硬契约（`BundleManifest::INNER_PAYLOAD_FILENAME`），导入侧 `extract_inner_payload` 按这个名字取回后喂给 `podman load -i` / `wsl --import`。

## 导出流程

CLI 是唯一实现（Tauri GUI spawn `clawcli export --json`，不重写）。按 backend 分：

1. **Native / Lima**：`BundleManifest::write_to_dir(&root)` → `tar czf bundle -C root manifest.toml <items>` → 删 manifest sidecar。
2. **Podman / WSL**：`podman save` / `wsl --export` 到 scratch `payload.tar` → `BundleManifest::wrap_with_inner_tar(payload, bundle)`（内部：写 manifest 到 work dir，rename payload 进 work dir，tar czf 打包两项）。

所有大文件 IO 走 `tokio::fs` 异步，不会阻塞 runtime（重要：WSL export 常见 GB 级别）。

## 导入流程

所有入口都共享一次 peek + 一次验证：

```
1. BundleManifest::peek_from_tarball(path)         # 从 .tar.gz 流式取 clawenv-bundle.toml，不解整包
2. 校验 schema_version ≤ SCHEMA_VERSION            # 不认识的版本拒绝
3. 校验 bundle.sandbox_type == host.sandbox_type   # 跨 backend 拒绝
4. 按 sandbox_type 分派到具体 importer：
     Native  → install_native::install_from_bundle (tar xzf -C ~/.clawenv)
     Lima    → LimaBackend::import_image (tar xzf 到 LIMA_HOME)
     Podman  → PodmanBackend::import_image (先 extract_inner_payload → podman load -i)
     WSL     → WslBackend::import_image   (先 extract_inner_payload → wsl --import)
5. 用 manifest.claw_type 直接写入 InstanceConfig — 不再 probe 每个 claw 的 version 命令
```

## 文件名约定

GUI 保存对话框建议的文件名格式：

```
{sandbox}-{arch}-{claw_type}-{YYYYMMDD-HHMMSS}.tar.gz
```

例：
- `lima-arm64-hermes-20260418-083300.tar.gz`
- `wsl2-aarch64-openclaw-20260418-083300.tar.gz`
- `macos-arm64-hermes-20260418-083300.tar.gz` （Native）

这不是强制的 — 用户可以任意改名；manifest 是权威信息源，文件名只帮用户一眼区分。

## Schema 演进规则

**加字段** 可以不 bump schema_version：

- 新字段必须 `#[serde(default)]`，旧 reader 读得过去就行
- 旧字段不能删、不能改类型

**删字段 / 改字段语义** 必须 bump schema_version。目前是 V1，未来到 V2 的具体做法：

1. `core/src/export/manifest.rs::SCHEMA_VERSION` += 1
2. `clawenv-bundle.toml` 里 `schema_version` 自动跟着变（由 `BundleManifest::build` 写入）
3. 新旧共存阶段用 `#[serde(untagged)]` 枚举分派多版本反序列化：

```rust
// 将来升级的模板
#[derive(Deserialize)]
#[serde(untagged)]
enum AnyManifest {
    V1(BundleManifestV1),
    V2(BundleManifestV2),
}

// peek_from_tarball 里：
let any: AnyManifest = toml::from_str(&text)?;
let m: BundleManifest = match any {
    AnyManifest::V2(m) => m,
    AnyManifest::V1(old) => old.migrate_to_v2(),  // 手写迁移
};
```

**约束：**
- V1 reader 必须永远能读 V1 bundle（forward compat）
- V2 reader 必须能读 V1 bundle（backward compat）
- V1 reader 遇到 V2 bundle 必须 bail 并提示升级（已经实现 — 见 `peek_bails_on_newer_schema` 测试）

两个单测 (`peek_accepts_current_and_older_schemas`, `peek_bails_on_newer_schema`) 钉住当前半边契约；V2 引入时要再加 migrate 往返测试。

## 本文件的测试覆盖

`core/src/export/manifest.rs::tests` 覆盖：

- `toml_roundtrip` / `parse_rejects_garbage` — schema 基本稳定性
- `peek_from_tarball_roundtrip` / `peek_from_tarball_bails_on_missing_manifest` — 读路径
- `wrap_and_extract_inner_payload` — Podman/WSL wrap/unwrap 往返
- `build_fills_defaults` — 构造函数默认值
- `sandbox_type_tests::wire_str_matches_serde` — `SandboxType::as_wire_str()` 与 serde 的 kebab-case 不漂移

外层 e2e（安装 → 启动 → 导出 → 重新导入 → 卸载）靠 macOS 手工跑，计划由 `scripts/e2e-bundle.sh` 覆盖。

## 参考

- 代码：`core/src/export/manifest.rs`
- CLI export：`cli/src/main.rs` 里 `Commands::Export` 的四个 backend 分支
- CLI import：`cli/src/main.rs` 里 `Commands::Import`
- backend 内部 importer：`core/src/sandbox/{lima,podman,wsl}.rs::import_image`
- Native 导入：`core/src/manager/install_native/mod.rs::install_from_bundle`
- GUI 胶水：`tauri/src/ipc/export.rs`（薄壳，spawn CLI）

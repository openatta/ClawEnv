# G — GUI 切 v2 + 主线归并迁移计划

**起点**: commit `eb00cdb`（v1 完整 + v2 通过 R3 Gate-0）
**终点**: v2 代码在 root，v1 全删，单一 implementation
**估时**: 6 周（含 browser HIL 完整移植）
**滚回点**: tag `v1-final-pre-gui-merge` @ `eb00cdb`（5 分钟回滚）

## 决策已敲定

| | 内容 |
|---|---|
| F1 | 立刻打 `v1-final-pre-gui-merge` ✅ |
| F2 | **browser HIL 完整移植**（chromium headless + noVNC HIL，Hermes 必备） |
| F3 | 先 G1 易切片，再补 R3.1 |
| 终态 | v2 上移到 root，v1 全删，**目录树只剩一份实现** |
| UI 新增 | 实例级 **诊断 / 升级 / 修复** 入口（CLI 已有，IPC 包一层即可） |

## 整体节奏（6 周）

```
Day 0           G0  打 tag + 文档落地（本会话）
Week 1          G1  Type A 命令换 binary（19 个 IPC + 3 新增 UI 入口）
Week 2-3        G2  v1 → v2 port verbatim（launcher/browser/update/...）
Week 4-5        G3  Tauri 重连：Cargo dep 切换、main.rs 启动序列、Type B IPC 重写
Week 6          G4  workspace 归并：v2/* → root，v1 删除
```

每个阶段结束打 tag，**任何阶段失败**都可 revert + 退回上个阶段 tag。

---

## Phase G0 — Foundations（本会话）

- [x] tag `v1-final-pre-gui-merge` @ `eb00cdb`
- [x] migration 文档落地（本文件）
- [ ] 增 `clawcli` 二进制别名（`v2/cli/Cargo.toml`）让 cli_bridge 找得到
- [ ] 第一批 G1 易切片（3-5 个 IPC 路由到 v2 binary）

## Phase G1 — Type A 命令换 binary（19 个，约 1 周）

**核心动作**: 改 `tauri/src/cli_bridge.rs` 的 binary 查找顺序，**优先找 v2 的 clawcli**，落不到再回 v1。所有 19 个 Type A IPC 不需要改一行代码——只是换了底层进程。

### 19 个 Type A 命令（已是 cli_bridge 委托）

| IPC | v2 等价 | 状态 |
|---|---|---|
| `install_openclaw` | `clawcli install <claw>` | ✅ R3-P3 |
| `system_check` | `clawcli doctor` | ✅ R3.1-c flag override |
| `list_instances` | `clawcli list` | ✅ R3 |
| `get_instance_logs` | `clawcli sandbox dump-logs` | ✅ R1-c |
| `start_instance` / `stop_instance` | `clawcli start/stop` | ✅ Phase 0 |
| `delete_instance` | `clawcli instance destroy` | ✅ R3 |
| `rename_instance` | `clawcli sandbox rename` | ⚠️ trait 有 `supports_rename`，CLI 没暴露 → 加 |
| `edit_instance_resources/ports` | `clawcli sandbox port {add,remove}` | ⚠️ resources 没有 → 加 `clawcli sandbox edit --cpus N` |
| `get_instance_health` | `clawcli status` | ✅ |
| `check_instance_update` | `clawcli upgrade --check` | ⚠️ check 模式没暴露 → 加 `--check` flag |
| `upgrade_instance` | `clawcli upgrade` | ✅ R4-a |
| `export_sandbox` / `export_native_bundle` | `clawcli export` | 🔴 R4-b 待做 |
| `list_sandbox_vms` | `clawcli sandbox list` | ⚠️ 没有 → 加 |
| `list_claw_types` | `clawcli claw list` | ✅ |

**G1 子任务**

- G1-a: cli_bridge 二进制查找加 v2 优先级
- G1-b: 补缺 CLI verb：`sandbox rename` / `sandbox edit` / `sandbox list` / `upgrade --check`
- G1-c: 19 个 Type A 全部走通 (smoke test pass)
- G1-d: **新增 3 个 UI IPC**（用户要求）：
  - `instance_diagnose` → `clawcli doctor <name>`
  - `instance_upgrade` → `clawcli upgrade <name>`（已存在但 UI 没 wire）
  - `instance_repair` → `clawcli sandbox repair <issues>...`
- G1-e: tag `v0.4.0-alpha1`

**风险**：v2 clawcli 输出 schema 跟 v1 不完全对齐——前端 schema 校验可能要 patch。

---

## Phase G2 — Lift v1 verbatim 到 v2（约 2 周）

**核心**: v1 那些跨平台抽象好的模块直接搬。每个模块 1 commit。

### 移植清单

| v1 路径 | 新 v2 路径 | 改动类型 | 工时 |
|---|---|---|---|
| `core/src/launcher.rs` | `v2/core/src/launcher/mod.rs` | 直搬，换 import：`clawenv_core::sandbox::SandboxBackend` → `crate::sandbox_backend::SandboxBackend`；`ConfigManager` → 新 `v2/core/src/config_loader::GlobalConfig`（已有） | 0.5 天 |
| `core/src/update/checker.rs` | `v2/core/src/update/mod.rs` | 直搬，零改（pure HTTP） | 0.5 天 |
| `core/src/browser/chromium.rs` | `v2/core/src/browser/mod.rs` | 直搬，依赖 `SandboxBackend::exec_with_progress` —— **v2 trait 需补一个** stream-friendly exec | 1 天 |
| `core/src/sandbox/lima.rs::ensure_prerequisites` 等 | `v2/core/src/install_native/{macos,windows,linux}.rs` | 直搬三平台分支 | 1 天 |
| `core/src/manager/instance.rs::stop_native_gateway / kill_native` | 拆进 `v2/core/src/native_ops/lifecycle.rs` | 直搬 | 0.5 天 |
| `core/src/bridge/...`（gateway_token 读取等） | 暂保留在 tauri/，等 G3 决定 | - | 0 |
| `tauri/src/ipc/bridge.rs` 的 `get_gateway_token` | 改为先 read in-VM file via v2 backend，否则保 v1 path | 0.5 天 |

### v2 SandboxBackend trait 扩展

加一个方法（兼容 v1 用法）：
```rust
/// Execute and stream stdout lines via mpsc channel. Used by long-
/// running operations that need progress (apk install, npm install).
/// Default impl wraps exec_argv with line-by-line tail polling.
async fn exec_argv_with_progress(
    &self,
    argv: &[&str],
    out: mpsc::Sender<String>,
) -> anyhow::Result<()>;
```

实际就是 `run_background_script` 的轻量版。

**G2 收尾**: tag `v0.4.0-alpha2`

---

## Phase G3 — Tauri 切到 v2（约 2 周）

### G3-a: Cargo 依赖切换

```diff
# tauri/Cargo.toml
- clawenv-core = { path = "../core" }
+ clawops-core = { path = "../v2/core" }
```

预计编译 break ≈ 200 处。每处改动是 import 路径 + 类型名映射。

### G3-b: main.rs 启动序列改写

5 处直调点改用 v2 等价：
- proxy_resolver::triple_from_config_proxy → v2 `proxy::Scope::Installer.resolve(...)` 
- proxy_resolver::apply_env → v2 新加 `proxy::apply_env(triple)`（G2 lift verbatim）
- launcher::detect_launch_state → v2 `launcher::detect_launch_state`（G2 已搬）
- update::checker → v2 `update::checker`（G2 已搬）
- ChromiumBackend status polling → v2 `browser::ChromiumBackend`（G2 已搬）

### G3-c: 28 个 Type B IPC 重写

按"直搬"原则一个个改。优先序：
1. settings / proxy / mirrors / config 类（用 v2 `config_loader` + `proxy` 模块）— 5 个
2. instance 类（用 v2 `InstanceOrchestrator`）— 6 个
3. browser HIL 三连（install/status/start/resume）— 3 个
4. sandbox 类（用 v2 `SandboxOps`）— 5 个
5. 杂项（diagnose / fix / has_native / get_capabilities）— 9 个

### G3-d: tray.rs 改 import（最简单，3 处）

### G3-e: 前端 schema 对齐

v2 `clawcli list --json` / `clawcli status --json` / `ProgressEvent` 输出对照 v1 老 schema，差异处加适配：
- v2 `ProgressEvent.percent: Option<u8>` → 前端 `percent ?? 0`
- v2 `MirrorsConfig` 没 `nodejs_dist` → 前端表单去掉
- v2 `InstanceConfig.dashboard_port` —— 待 G2 期间补到 v2 InstanceConfig（v1 有这字段）

**G3 收尾**: tag `v0.4.0-rc1`，全功能跑一轮 smoke

---

## Phase G4 — 归并主线（约 3 天）

### G4-a: 删 v1

```bash
git rm -r core/ cli/
# tauri/ 保留
```

### G4-b: v2 上移

```bash
mv v2/core .            # → core/
mv v2/cli .             # → cli/
mv v2/assets/* assets/  # 合并 assets/
mv v2/docs/* docs/      # 合并 docs/
rm -rf v2/
```

### G4-c: workspace 重排

```diff
# Cargo.toml (root)
[workspace]
resolver = "2"
- members = ["core", "tauri", "cli"]   # v1 顺序
+ members = ["core", "cli", "tauri"]   # 一致
```

```diff
# tauri/Cargo.toml
- clawops-core = { path = "../v2/core" }
+ clawops-core = { path = "../core" }
```

### G4-d: 单元名重整

- crate `clawops-core` 保留还是改回 `clawenv-core`？**保留 clawops-core**（一致性，生命周期清楚）
- bin `clawcli` 保留 ✅
- bin `clawgui` 保留 ✅

### G4-e: CLAUDE.md 更新

把 "v1 是 ...，v2 是 ..." 这类区分话语全去掉，改成单一架构描述。

### G4-f: tag `v0.4.0`

---

## 滚回策略（重要）

每个阶段失败 → revert 到该阶段开始的 tag，全程 5 分钟内可恢复。

| 阶段失败 | 退回 |
|---|---|
| G1 失败 | `v1-final-pre-gui-merge` |
| G2 失败 | `v0.4.0-alpha1` |
| G3 失败 | `v0.4.0-alpha2` |
| G4 失败 | `v0.4.0-rc1`（v1 已删，但 v2 + Tauri 都通过 G3 验证） |

---

## 不要做的事

- ❌ 不要在 G1/G2 中混着改 Tauri 接口 —— G3 才碰
- ❌ 不要边搬边重构（Lift verbatim 阶段要纯直搬）
- ❌ 不要跳过 alpha tag（每阶段必 tag，万一回滚要找得到点）
- ❌ G4 之前不要删任何 v1 代码

---

## R3.1 / R4-b/c 待做的关系

| 项 | 何时做 |
|---|---|
| R3.1-a Hermes dashboard 预构建 | G2 期间穿插（dashboard 是 install 的子步） |
| R3.1-b MCP plugin deployment | G3 期间随 instance IPC 一起 |
| R4-b export | G3 之前必做（IPC export_sandbox 依赖） |
| R4-c import | G3 之前必做（IPC pick_import_folder 依赖） |
| auto-start gateway daemon | G2 期间补（install 完成后用户期望立刻可访问） |

---

**作者**: 落地于 `eb00cdb` 时点。下次会话起点。

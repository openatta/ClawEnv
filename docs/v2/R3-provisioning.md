# R3 — Provisioning 设计备忘

**状态**: 待决策。代码未动。
**基于**: commit `cb057a4` (R1+R2 完成态)
**工时估算**: 3–5 天，分 4 阶段

---

## 为什么单独立项

R1/R2 都是**确定性改造**（把 trait 改安全、把 proxy 从 v1 搬过来）。
R3 不是改造——它是**功能新增**：v2 第一次具备从零装出一个可用沙盒的能力。
这个功能在 v1 里是 8 步 836 行（`core/src/manager/install.rs`），牵涉 cloud-init 模板、
mirror 注入、post-boot 重装、npm install 长任务轮询、dashboard 预构建、MCP 注册
—— 必须先定好边界再动手，不然会做成 v1 复印件，失去 v2 的架构优势。

---

## R3 要做的事（完整拆解）

基于对 v1 install.rs、background.rs、mirrors.rs、clawenv-alpine.yaml 和
CHANGELOG v0.2.8–v0.3.2 的深读，完整 R3 包含 8 块：

| # | 模块 | v1 位置 | v2 现状 | 工时 |
|---|---|---|---|---|
| 1 | **Mirrors apply**（在 VM 内 `sudo tee /etc/apk/repositories` + `npm config set registry`） | `core/src/config/mirrors.rs:17-73` | 无 | 0.5 天 |
| 2 | **Cloud-init 模板 + Containerfile**（Lima YAML + Podman Containerfile + WSL first-boot 脚本，内含 apk 基础包、ssh-keygen、proxy 内联） | `assets/lima/clawenv-alpine.yaml`、Podman Containerfile | 无 | 1 天 |
| 3 | **Post-boot 重装**（CHANGELOG v0.2.12 的关键修复：cloud-init 可能静默失败，探 npm/git/curl 存在，缺失则 3 次重试 apk add） | `core/src/manager/install.rs:519-551` | 无 | 0.5 天 |
| 4 | **Background script 框架**（nohup + `/tmp/clawenv-install.{log,done}` 轮询，超时 1200s，进度按 elapsed 映射到 pct 区间） | `core/src/manager/background.rs:63-156` | 无 | 1 天 |
| 5 | **Claw 描述符**（ClawDescriptor：sandbox_provision 包列表、sandbox_install_cmd、version_check_cmd、has_dashboard、mcp_plugins） | `core/src/claw/*` | 只有 ClawRegistry+hermes/openclaw CLI 命令生成 | 0.5 天 |
| 6 | **Dashboard 预构建**（Hermes：写 `.env`、`pip install --break-system-packages fastapi uvicorn`、chown `/opt/hermes`、`npm install + npm run build`；失败不 fatal） | `core/src/manager/install.rs:597-646` | 无 | 0.5 天 |
| 7 | **MCP plugin deploy**（把 plugin 脚本内嵌进 VM、用 registry 注册） | `core/src/manager/install.rs:649-734` | 无 | 0.5–1 天 |
| 8 | **Orchestrator**（把 1–7 连起来成 8-stage install pipeline，挂 ProgressSink 出事件） | `core/src/manager/install.rs:139-836` 骨架 | `v2/core/src/instance/orchestrator.rs` 只做 preflight+port+记录 | 0.5 天 |

**合计**：约 5 天。

---

## 需要你先决策的 4 件事

### D1. **v2 要不要管 VM 创建？**

这是最关键的分岔。v2 当前 `SandboxBackend` trait 的文档明确说：

> VM creation/destruction and rootfs import are out of scope — those are
> one-time bootstrap operations and v1's installer already handles them.
> v2 manages the *running* VM.
> —— `v2/core/src/sandbox_backend/mod.rs:1-8`

但 provisioning 和 VM 创建**物理上不可分**：Lima 的 apk 装包发生在
`cloud-init runcmd:` 里，是 VM 第一次启动时；WSL 的 rootfs import 后马上就要装包；
Podman 的 Containerfile `RUN apk add` 是 image build 期。你不能只做 "post-boot
provision"，Lima 的 cloud-init 必然要走 v2 的模板生成。

三条出路：

- **A. v2 全包（推荐）**：取消"v2 不管 VM 创建"的限制；v2 的 SandboxBackend 加 `create()/destroy()`；assets 模板搬进 `v2/assets/lima|podman|wsl/`；工时按上表 5 天。
  - **优点**：v2 第一次真正有了 end-to-end 安装能力，可以独立替代 v1 的 install.rs。
  - **代价**：assets 复制一份（不是 share；因为 v1 模板仍要用）；trait 表面变大。

- **B. v2 只做 post-boot provision，VM 创建仍让 v1 做**：v2 暴露 `provision_existing_vm()`，假定调用方先用 v1 的 installer 把 VM 起来。
  - **优点**：工时 -1 天（跳过模块 2），trait 不变。
  - **代价**：v2 永远依赖 v1，DESIGN § 阶段 D "v1 废弃" 永远做不到。
  - **适用**：短期只想给 v2 补上 v1 缺的运维能力，不想替代 v1。

- **C. 两条路都开**：provision trait 可以走两条路径——"v2 自建 VM 后 provision" 和 "复用 v1 建好的 VM provision"。通过 `ProvisionContext` 枚举区分。
  - **优点**：迁移期友好，可以灰度。
  - **代价**：多写一倍分支代码。

**我的建议**：走 **A**。B 会把 v2 锁死在 v1 的附庸位置；A 多 1 天工时但买断自由。

### D2. **ClawDescriptor 要不要完整移植？**

v1 的 `ClawDescriptor` 有这些字段（`core/src/claw/mod.rs`）：

- `id` / `display_name` / `binary`
- `sandbox_provision: Vec<String>` — 这个 claw 额外需要的 apk 包（Hermes 要 python3-dev）
- `sandbox_install_cmd: fn(version) -> String` — npm/pip 安装命令
- `version_check_cmd: fn() -> String`
- `supports_native: bool`
- `has_dashboard: bool` + `dashboard_build_steps` — Hermes 独有
- `mcp_plugins: Vec<McpPluginSpec>` — 要部署的 MCP 插件

v2 的 `ClawCli` trait 只有 CLI 命令生成器（update/doctor/config/logs/status/version）。
install 需要的 descriptor 字段完全没有。

**选项**：

- **1. 完整移植**：在 v2 加一个 `ClawDescriptor` 结构，ClawCli impl 也带描述符。hermes/openclaw 两个 impl 补齐所有字段。
- **2. 渐进移植**：只先做 install 用到的字段，dashboard/MCP 先留空，用一层 `Optional<DashboardSpec>` 等。
- **3. 分开 trait**：`ClawCli`（CLI 表面）和 `ClawProvisioning`（装包描述）分成两个 trait。

**我的建议**：**3**。两个关注点真的独立——`ClawCli` 只负责生成命令，`ClawProvisioning` 负责 install/upgrade。拆开让未来新增 claw 不需要都实现两套。

### D3. **Dashboard + MCP 是不是 R3 必须？**

- Dashboard 预构建只对 Hermes 需要，非 fatal（失败首次启动 rebuild）。
- MCP plugin 部署目前 v1 用得不多。

**选项**：

- **1. 一把梭**：R3 就做完。
- **2. R3 做基线，dashboard/MCP 留 R3.1**：R3 先让最小路径（apk+npm install+startup）跑通，hermes 能 `clawops instance create --claw hermes` 装出来但 dashboard 先不 pre-build。
- **3. 完全不做 dashboard/MCP**：v2 只做 headless install；dashboard/MCP 留给用户自己或 v1。

**我的建议**：**2**。R3 基线 4 天，R3.1 按需做 1 天。先验证 end-to-end 跑通再补特性。

### D4. **config.toml 加载要不要现在做？**

v2 目前没有任何全局配置加载。provisioning 需要读配置的地方：

- ProxyConfig（R2 已做数据模型，但没加载代码）
- InstanceRegistry（已做，但和 v1 config.toml 格式不共享）
- 每个 instance 的 installed state（version、gateway port、install timestamp）

**选项**：

- **1. 复用 v1 config.toml**：v2 直接 parse `~/.clawenv/config.toml`（v1 写的那个）。
- **2. v2 另起 `~/.clawenv/v2-config.toml`**：和 v1 分开。
- **3. 先不做持久化**：R3 只走内存 + CLI 传参；config 加载留 R4。

**我的建议**：**1**。v1/v2 共用 config.toml 是让用户看得到"v2 装的也是我的 clawenv" 的关键体验。
字段冲突可以通过 `#[serde(default)]` + 向前兼容新字段解决。这加 0.5 天（解析 + 原地合并写）。

---

## 推荐的分波次方案

基于上面的决策（A/3/2/1）：

### R3-波次 1 — 基础设施（1.5 天）

- **R3a**: `v2/core/src/provisioning/mirrors.rs`
  - `async fn apply_mirrors(backend, mirrors_cfg) -> Result<()>` —— 写 `/etc/apk/repositories`（sudo tee，经 `exec_argv`）+ `npm config set registry`
  - mirrors.toml loader（搬 `core/src/config/mirrors_asset.rs`）
  - 单测：Alpine version 探测、仓库行拼接、用户 override 优先级

- **R3b**: `v2/core/src/provisioning/background.rs`
  - `async fn run_background_script(backend, script, opts) -> ProgressStream`
  - nohup + `/tmp/clawenv-install.{log,done}` 轮询模型
  - idle_timeout=1200s, tail 间隔=5s, exit_code 解析
  - 单测：用 MockBackend 模拟 log 写入 + done marker，验证 progress 事件流

- **R3c**: `v2/core/src/provisioning/preflight.rs` —— VM 内 curl preflight
  - 已有 `v2/core/src/preflight/mod.rs` 是 host 侧；sandbox 侧再加一个从 VM 内 curl 3 点（扩展现有 module）

### R3-波次 2 — VM 创建 + cloud-init（1.5 天）

- **R3d**: 把 `assets/lima/clawenv-alpine.yaml` 复制到 `v2/assets/lima/clawenv-alpine.yaml`，改成模板变量形式（`{PROXY_SCRIPT}`/`{USER_ALPINE_MIRROR}`/`{PACKAGES}`）
- **R3e**: 把 `assets/podman/Containerfile` 同样搬
- **R3f**: `v2/core/src/provisioning/templates.rs` —— 渲染逻辑（handlebars？还是纯字符串替换？v1 是纯替换，500 LOC 简洁）
- **R3g**: `SandboxBackend` trait 扩：

  ```rust
  async fn create(&self, opts: CreateOpts) -> Result<()>;
  async fn destroy(&self) -> Result<()>;
  ```

  CreateOpts 含 cpus/memory_gb/proxy_script/alpine_mirror/npm_registry/packages。
- **R3h**: 三个 backend 实装 create/destroy（Lima: `limactl create --tty=false ...`；WSL: `wsl --import`；Podman: `podman build` + `podman run`）
- **R3i**: 单测：模板渲染 golden tests；`create()` 只测到"命令参数拼对了"这一层，不真起 VM

### R3-波次 3 — Claw descriptor + orchestrator（1.5 天）

- **R3j**: `v2/core/src/claw_ops/descriptor.rs` —— `ClawProvisioning` trait

  ```rust
  pub trait ClawProvisioning: Send + Sync {
      fn sandbox_provision_packages(&self) -> &[&str]; // 额外 apk 包
      fn install_cmd(&self, version: &str) -> String;
      fn version_check_cmd(&self) -> String;
      fn supports_native(&self) -> bool;
  }
  ```

  hermes.rs / openclaw.rs 两个 impl。

- **R3k**: `v2/core/src/instance/orchestrator.rs` 扩展为完整 8-stage install pipeline（v2 的 stages 和 v1 对齐：DetectBackend → EnsurePrereq → CreateVm → BootVm → ConfigureProxy → InstallDeps → InstallClaw → SaveConfig → Complete）。每 stage 用 `ProgressSink` 发事件。post-boot verify（v0.2.12 那段）嵌在 InstallDeps 之前。

- **R3l**: CLI：`clawops instance create --claw X --backend Y --version Z`（现有 stub 扩展）

- **R3m**: 集成测试：用 `MockBackend` + fake script log 驱动，端到端跑 8 stages，断言进度事件序列

### R3-波次 4 — config.toml 共享 + 收尾（0.5 天）

- **R3n**: `v2/core/src/config_loader/` —— 读 v1 格式的 `~/.clawenv/config.toml`，暴露 `ClawEnvConfig { proxy: ProxyConfig, instances: Vec<InstanceConfig>, mirrors: MirrorsConfig }`
- **R3o**: orchestrator 写 config.toml（合并式：load → update 对应 instance → write with lock）
- **R3p**: clippy + full test + checkpoint commit

**Dashboard + MCP**（留 R3.1，约 1 天）：等基线跑通、有个能装出来的 hermes 实例后，再补 `dashboard_build` + `mcp_plugins`。

---

## 风险 & 防护

| 风险 | 表现 | 防护 |
|---|---|---|
| Lima cloud-init 模板语法错误 | VM 起不来，报错信息吞掉 | golden test 对齐 v1 模板字节级；CI 加 "`limactl validate`" 如果可能 |
| Background script 进度不动 | npm install 超时 20min 被 kill | 复制 v1 的 idle_timeout=1200s + elapsed→pct 线性映射 |
| post-boot verify 判断错 | 包其实在但 test failed，重装触发坏副作用 | 走和 v1 完全一样的 shell-test 短路链路（`npm >/dev/null && git >/dev/null && echo OK`） |
| sudo 在 WSL 默认 root 下炸 | WSL 的 provision 用户是 root，没有 sudo | backend 层适配：`exec_argv(["sudo", ...])` 在 WSL root 上直接剥掉 sudo |
| GFW 环境 apk mirror 不通 | provision 卡住 | v0.3.0 铁律 —— 上游 + proxy，不做兜底。v2 preflight 已做，失败即拒 |
| MCP plugin 脚本内嵌文件太大 | 编译产物膨胀 | 用 `include_bytes!` + lazy decompression；或者 CLI 推 plugin 时从 install dir 读 |
| v1 和 v2 同时写 config.toml 竞争 | 并发 write race | 加文件锁（`fs2::FileExt::lock_exclusive`）或 load-modify-write with etag |

---

## 前提（不做不能开工）

- [ ] **你确认 D1（v2 全包 VM 创建）**——决定了整个 scope
- [ ] **你确认 D2/D3/D4 的选项**——决定了 trait 形状和 config 策略
- [ ] 你 review 一下 commit `cb057a4`，确认 R1/R2 没大问题，R3 才在它上面继续

---

## 一开始的最小可验证里程碑（R3 Gate 0）

真正动手第一天就应该能看到：

1. `clawops instance create --claw openclaw --backend lima --name test`
2. 屏幕上实时滚出 8 stage 进度（和 v1 install 一样）
3. 结束后 `clawops sandbox status --instance test` 看到 VM running
4. `limactl shell test` 进去看到 `openclaw --version` 能跑

如果这个 gate 过了，后面加 hermes、dashboard、MCP 都是增量。如果 gate 过不了，说明
orchestrator 或 provisioning 骨架有问题，及早收手比继续往上堆好。

---

## 不在 R3 范围的事

以下留给 R4+ 或单独议题：

- **upgrade pipeline**（v1 的 upgrade.rs）——和 install 共享 80% 代码但有差异；R3 先做 install
- **export/import**（v1 的 export instance 导出镜像）——独立功能
- **Tauri IPC 接口**（DESIGN § 阶段 C "Tauri 转发到 clawops 子进程"）——等 R3 稳了之后可以做
- **Linux native mode 的 provision**（v1 有但 CLAUDE.md 说 Linux GUI 不维护）——随便

---

**等你对 D1–D4 给决策，我开 R3-波次 1**。

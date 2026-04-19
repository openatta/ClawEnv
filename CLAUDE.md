# ClawEnv

OpenClaw（及 claw 生态）的跨平台沙盒安装器、启动器与管理器。

## 技术栈

- **后端**: Rust 2021 edition
- **GUI**: Tauri v2（系统原生 WebView）—— **仅 macOS + Windows**，Linux 不做 GUI 支持
- **前端**: SolidJS + TailwindCSS v4 + TypeScript
- **CLI**: clap v4（derive 模式）—— 三平台都支持（含 Linux）
- **配置**: TOML（`~/.clawenv/config.toml`）
- **沙盒**: Alpine Linux，三平台对等后端（WSL2 / Lima / Podman）+ Native（开发者模式）

## 平台支持矩阵

|        | macOS | Windows | Linux |
|--------|-------|---------|-------|
| CLI    | ✅     | ✅       | ✅     |
| Sandbox| ✅ Lima | ✅ WSL2 | ✅ Podman |
| GUI    | ✅     | ✅       | ❌ 不支持 |

**Linux GUI 明确不支持**：现存的 Linux GUI 相关代码（例如 `install_native/linux.rs` 的 Node 安装、`SandboxPage` 的 Podman 路径等）保留但不主动维护、不为其做新特性适配。新增 GUI 功能只保证 macOS + Windows 双平台同步，Linux 侧维持现状即可，不需要清理。Linux 用户通过 CLI（`clawcli`）使用。

## Workspace 结构

```
core/            # 核心逻辑（平台无关，无 UI 依赖）
tauri/           # Tauri GUI 应用（含 System Tray）
cli/             # 纯 CLI（开发者模式）
src/             # 前端 SolidJS
assets/          # 平台模板、图标资源
docs/            # 规格文档（SSOT，共 11 个文件）
```

## 架构铁律

1. **沙盒后端对等**：WSL2 / Lima / Podman 是同一层级的三种对等实现，`detect_backend()` 工厂函数只返回一个后端，不做组合，不嵌套。
2. **Tauri IPC 异步**：安装、升级等耗时操作必须通过 `tauri::Emitter::emit` 推送事件，不能用同步 IPC。
3. **凭证安全**：API Key、代理密码一律存入系统 Keychain（`keyring` crate），`config.toml` 和日志中不得出现明文。
4. **浏览器安全边界**：Chromium 必须安装在沙盒内部，不得调用宿主机浏览器。noVNC 仅传输画面像素流。
5. **启动器路由**：`App.tsx` 的 `LaunchState` 状态机是唯一顶层路由入口，不得在组件内直接跳转。
6. **System Tray 生命周期**：托盘在 Tauri `setup` 阶段初始化，不依赖主窗口。主窗口关闭时继续在托盘运行。
7. **Claw 管理页是 WebView**：不自己实现管理 UI，内嵌加载各 Claw 产品自带的 Web 管理面板。ClawPage 组件通过 ClawDescriptor 动态适配不同产品。

8. **CLI 是核心**：所有业务逻辑通过 CLI（`clawcli --json`）暴露，Tauri GUI 是薄壳，通过 `cli_bridge` spawn CLI 子进程。
9. **Shell 安全**：所有拼入 shell 的动态变量必须用 `shell_quote()` / `powershell_quote()` 转义（`core/src/platform/mod.rs`）。
10. **Bridge 是独立 daemon**：AttaRun bridge 和沙盒实例平级，由系统守护机制（launchd/systemd/Task Scheduler）托管。ClawEnv 通过 admin API（HTTP `127.0.0.1`）和守护协议管理，不做父进程。ClawEnv tray 关闭不影响 bridge 在线状态。详见 `docs/22-attarun-bridge.md`。

## Rust 版本

- **core + cli**: rustc 1.87+ (Homebrew)
- **tauri**: rustc 1.88+ (需 `~/.cargo/bin/rustc`，Tauri 依赖 darling/time 要求)
- 构建 Tauri 时需 `export PATH="$HOME/.cargo/bin:$PATH"`

## 开发命令

```bash
cargo tauri dev          # 开发模式（热重载）
cargo tauri build        # 生产构建
cargo test --workspace   # 运行所有测试
cargo clippy --workspace # Lint
npm install              # 前端依赖
```

## 规格文档

所有设计决策的 SSOT 在 `docs/` 目录，开发前必须阅读相关文档：

- `docs/README.md` — 文档索引
- `docs/02-architecture.md` — 沙盒架构（最重要）
- `docs/04-sandbox.md` — 三平台实现 + 浏览器 + noVNC
- `docs/05-launcher.md` — 启动流程状态机
- `docs/06-main-ui.md` — Slack 风格 UI 布局
- `docs/09-config.md` — 配置格式 + 源码结构

## 关键约束

- Lima cgroup v2：模板中必须包含 `sed -i 's/rc_cgroup_mode=.*/rc_cgroup_mode=unified/' /etc/conf.d/cgroups`
- Podman rootless：`podman run` 必须加 `--userns=keep-id`，volume 加 `:Z`
- 代理密码安全：`config.toml` 仅存用户名，密码存 Keychain，沙盒内通过环境变量注入
- 安装模式：在线构建 / 预构建镜像下载 / 本地镜像文件导入

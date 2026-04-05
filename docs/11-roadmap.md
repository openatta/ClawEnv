# 11. 开发路线图与开发指引

## 11.1 路线图

**Phase 1 — 核心安装器 + 启动器（MVP）**
三平台沙盒后端（含 Native 模式）、双安装模式（在线构建/预构建镜像）、启动检测状态机、
GUI 安装向导（含代理配置）、Slack 风格主 UI（Home + OpenClaw WebView + 设置）、
沙盒内 Chromium headless + noVNC 人工介入、普通用户/开发者模式切换、基础 CLI、
Keychain 集成、System Tray 常驻。

**Phase 2 — 升级与浏览器**
版本检查（GitHub Releases API）、CVE 监控、一键升级 + 自动快照、手动回滚、CDT 浏览器集成。

**Phase 3 — 多实例与开发工具**
多实例管理、快照管理 UI、`clawenv skill` 工具链、性能监控、完整日志查看器、用户登录（对接 Atta Cloud）。

**Phase 4 — 高级功能**
指纹浏览器集成（Camoufox 优先）、ZeroClaw / NanoClaw 支持、Skill 发布到官方 Registry、ClawEnv 自更新。

---

## 11.2 环境搭建

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust
cargo install tauri-cli --version "^2"                             # Tauri CLI
npm install                                                        # 前端依赖
cargo tauri dev                                                    # 开发模式
cargo tauri build                                                  # 生产构建
```

## 11.3 任务分解建议

| 任务 | 文件 | 关键点 |
|---|---|---|
| 1. 项目脚手架 | Cargo workspace + `tauri/tauri.conf.json` | 三 crate 结构 |
| 2. 平台检测 | `core/src/platform/detector.rs` | 返回 `PlatformInfo`，决定使用哪个后端 |
| 3. SandboxBackend trait | `core/src/sandbox/mod.rs` | 三后端对等，工厂函数自动选择 |
| 4. WslBackend | `core/src/sandbox/wsl.rs` | 优先开发，Windows 用户最多 |
| 5. LimaBackend | `core/src/sandbox/lima.rs` | 注意 cgroup v2 fix |
| 6. PodmanBackend | `core/src/sandbox/podman.rs` | rootless + userns=keep-id |
| 7. 启动器状态机 | `core/src/launcher.rs` + `src/App.tsx` | 检测安装/升级状态，路由到正确页面 |
| 8. 安装流程 | `core/src/manager/install.rs` + `src/pages/Install/index.tsx` | 通过 Tauri 事件 emit 进度，前端 7 步合并实现 |
| 9. 主 UI 布局 | `src/layouts/MainLayout.tsx` + `src/components/IconBar.tsx` | Slack 风格，左侧 56px 图标栏 |
| 10. Home 页 | `src/pages/Home.tsx` | Dashboard：实例卡片 + 健康状态 + 操作按钮 |
| 11. OpenClaw 页 | `src/pages/OpenClawPage.tsx` | 内嵌 WebView 加载 OpenClaw 管理面板（Tab 栏 + iframe） |
| 12. 设置页 | `src/pages/Settings.tsx` | 模式/语言/主题/托盘/更新配置，读写 config 持久化 |
| 13. System Tray | `tauri/src/tray.rs` | 托盘图标/菜单/通知，Tauri tray API |
| 14. IPC 命令 | `tauri/src/ipc/mod.rs` | 统一 IPC 模块，暴露安装/实例/升级等命令给前端 |
| 15. 实例监控 | `core/src/monitor.rs` | 后台轮询沙盒进程状态，驱动托盘刷新 |
| 16. 升级 + 版本检查 | `core/src/update/checker.rs` + `core/src/manager/upgrade.rs` | GitHub Releases API + 升级/回滚 |
| 17. 浏览器集成 | `core/src/browser/mod.rs` + `core/src/browser/chromium.rs` | 沙盒内 Chromium + noVNC，BrowserBackend trait |
| 18. 代理配置 | `core/src/config/proxy.rs` + `src/pages/Install/index.tsx` | 安装向导代理 UI + 沙盒内代理注入 |
| 19. 预构建镜像 | `core/src/manager/install.rs` | GitHub Releases 镜像下载 + 导入，离线安装支持 |
| 20. 升级提示 | `src/components/UpgradePrompt.tsx` | 升级提示弹窗覆盖层 |
| 21. noVNC 面板 | `src/components/NoVncPanel.tsx` | 人工介入面板（登录/CAPTCHA/OAuth） |
| 22. 配置模型 | `core/src/config/models.rs` | BrowserConfig, ChannelsConfig 等配置数据结构 |

## 11.4 关键约束

1. **架构铁律**：三个后端（WSL2/Lima/Podman）是对等关系，任何时候都不能出现后端嵌套。
   `detect_backend()` 工厂函数只返回一个后端，不做组合。

2. **Tauri IPC 异步**：安装、升级等耗时操作必须通过 `tauri::Emitter::emit` 推送事件给前端，
   不能用同步 IPC（会阻塞 UI）。

3. **凭证安全铁律**：任何情况下 API Key 不得写入 `config.toml`、日志或进程环境变量可见位置。
   统一通过 `keyring` crate 操作系统 Keychain。

4. **Lima cgroup v2**：`clawenv-alpine.yaml` 模板中必须包含
   `sed -i 's/rc_cgroup_mode=.*/rc_cgroup_mode=unified/' /etc/conf.d/cgroups`，
   否则 Alpine VM 内资源控制不完整。

5. **Podman rootless**：`podman run` 必须加 `--userns=keep-id`，
   volume 挂载必须加 `:Z` SELinux 标签（在 SELinux 系统上必需，非 SELinux 系统无害）。

6. **System Tray 生命周期**：托盘必须在 Tauri `setup` 阶段初始化（`setup_tray`），
   不依赖主窗口存在。主窗口关闭时应用继续在托盘运行，点击"退出"才真正退出进程。
   `InstanceMonitor` 作为后台 tokio task 启动，通过 Tauri 事件驱动托盘刷新，
   不得在 UI 线程轮询。

7. **启动器路由**：`App.tsx` 中 `LaunchState` 状态机是唯一的顶层路由入口。
   不得在组件内部直接跳转到安装��导或主界面，必须通过状态机切换。

8. **OpenClaw 页是 WebView**：OpenClaw 管理页不自己实现 UI，而是内嵌加载 OpenClaw
   自带的 Web 管理面板。ClawEnv 只负责容器化运行和状态管理。

9. **浏览器安全边界**：Chromium 必须安装在沙盒内部，不得调用宿主机浏览器。
   noVNC 仅传输画面像素流，不传输 cookie 或会话数据。端口 6080 只绑定沙盒内 127.0.0.1。

10. **代理密码安全**：代理认证密码存入系统 Keychain（与 API Key 同策略），
    `config.toml` 中仅存用户名，不存密码明文。沙盒内通过环境变量注入（含密码），
    环境变量仅对沙盒内进程可见。

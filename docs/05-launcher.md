# 5. 启动器与路由

ClawEnv 同时作为**启动器（Launcher）**，每次启动时自动检测环境状态，
按条件引导用户进入不同流程，无需手动选择。

## 5.1 启动流程状态机

```
                        ClawEnv 启动
                            │
                    ┌───────▼───────┐
                    │  读取配置文件   │
                    │ ~/.clawenv/    │
                    │ config.toml    │
                    └───────┬───────┘
                            │
                   config.toml 存在？
                     │            │
                    NO           YES
                     │            │
              ┌──────▼──────┐    │
              │  首次运行    │    │
              │  模式选择    │    │
              │ 普通/开发者  │    │
              └──────┬──────┘    │
                     │           │
                     ▼           ▼
              ┌─────────────────────┐
              │  检测 OpenClaw 安装  │
              │  状态               │
              └──────────┬──────────┘
                         │
          ┌──────────────┼──────────────┐
          │              │              │
       未安装        已安装/已停止    已安装/运行中
          │              │              │
          ▼              ▼              ▼
    ┌───────────┐  ┌──────────┐  ┌──────────────┐
    │ 安装向导   │  │ 检查升级  │  │  检查升级     │
    │ (7步)     │  │          │  │              │
    └─────┬─────┘  └────┬─────┘  └──────┬───────┘
          │             │               │
          │        有升级可用？      有升级可用？
          │         │       │        │       │
          │        YES     NO      YES     NO
          │         │       │        │       │
          │         ▼       │        ▼       │
          │  ┌──────────┐   │ ┌──────────┐   │
          │  │ 升级提示  │   │ │ 升级提示  │   │
          │  │ 弹窗     │   │ │ 弹窗     │   │
          │  └────┬─────┘   │ └────┬─────┘   │
          │       │         │      │         │
          ▼       ▼         ▼      ▼         ▼
        ┌─────────────────────────────────────┐
        │           进入主界面                  │
        │      (Slack 风格布局)                │
        └─────────────────────────────────────┘
```

## 5.2 启动检测逻辑

```rust
// core/src/launcher.rs

/// 启动状态——决定进入哪个页面
#[derive(Debug, Clone)]
pub enum LaunchState {
    /// 首次运行，无配置文件
    FirstRun,
    /// OpenClaw 未安装（有配置但无实例）
    NotInstalled,
    /// 已安装，有可用升级
    UpgradeAvailable {
        instances: Vec<ClawInstance>,
        upgrade_info: VersionInfo,
    },
    /// 已安装，一切正常，直接进入主界面
    Ready {
        instances: Vec<ClawInstance>,
    },
}

/// 启动检测——在 Tauri setup 阶段调用
pub async fn detect_launch_state() -> Result<LaunchState> {
    // 1. 检查配置文件是否存在
    let config_path = dirs::home_dir()
        .ok_or(anyhow!("无法获取 home 目录"))?
        .join(".clawenv/config.toml");

    if !config_path.exists() {
        return Ok(LaunchState::FirstRun);
    }

    // 2. 读取配置，获取实例列表
    let config = ConfigManager::load()?;
    let instances = config.instances();

    if instances.is_empty() {
        return Ok(LaunchState::NotInstalled);
    }

    // 3. 检查升级（后台快速检查，超时 3 秒）
    match tokio::time::timeout(
        Duration::from_secs(3),
        check_upgrade(&instances[0])
    ).await {
        Ok(Ok(info)) if info.latest > info.current => {
            Ok(LaunchState::UpgradeAvailable {
                instances,
                upgrade_info: info,
            })
        }
        _ => Ok(LaunchState::Ready { instances }),
    }
}
```

## 5.3 前端路由映射

```typescript
// src/App.tsx

import { createSignal, onMount, Match, Switch } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type LaunchState =
  | { type: "loading" }
  | { type: "first_run" }
  | { type: "not_installed" }
  | { type: "upgrade_available"; instances: Instance[]; upgradeInfo: VersionInfo }
  | { type: "ready"; instances: Instance[] };

export default function App() {
  const [state, setState] = createSignal<LaunchState>({ type: "loading" });

  onMount(async () => {
    const result = await invoke<LaunchState>("detect_launch_state");
    setState(result);
  });

  return (
    <Switch fallback={<SplashScreen />}>
      <Match when={state().type === "first_run"}>
        <ModeSelect onComplete={() => setState({ type: "not_installed" })} />
      </Match>
      <Match when={state().type === "not_installed"}>
        <InstallWizard onComplete={(instances) =>
          setState({ type: "ready", instances })
        } />
      </Match>
      <Match when={state().type === "upgrade_available"}>
        <UpgradePrompt
          state={state() as any}
          onSkip={() => setState({
            type: "ready",
            instances: (state() as any).instances
          })}
          onUpgraded={(instances) => setState({ type: "ready", instances })}
        />
      </Match>
      <Match when={state().type === "ready"}>
        <MainLayout instances={(state() as any).instances} />
      </Match>
    </Switch>
  );
}
```

## 5.4 升级提示弹窗

升级提示不阻断用户使用，而是以模态弹窗形式出现，用户可选择：

```
┌──────────────────────────────────────────┐
│                                          │
│   OpenClaw 更新可用                       │
│                                          │
│   当前版本: v2.1.3                        │
│   最新版本: v2.1.4                        │
│                                          │
│   更新内容:                               │
│   - 修复 CVE-2026-25253 安全漏洞          │
│   - 改进 Telegram 消息解析性能            │
│                                          │
│   ┌─────────────┐  ┌──────────────────┐  │
│   │  立即更新    │  │  稍后提醒（跳过） │  │
│   └─────────────┘  └──────────────────┘  │
│                                          │
│   □ 以后自动更新，不再提示                │
│                                          │
└──────────────────────────────────────────┘
```

**行为规则**：
- 安全更新（含 CVE）：弹窗标题变红，强调安全风险，但仍允许跳过
- 普通更新：正常样式，可跳过
- 用户勾选"自动更新"：写入 `config.toml` 的 `[clawenv.updates] auto_upgrade = true`
- 跳过后直接进入主界面，托盘菜单中保留更新提示入口

## 5.5 安装后自动启动 OpenClaw

安装向导完成后，ClawEnv 自动：
1. 启动沙盒（`backend.start()`）
2. 启动 OpenClaw 进程（`backend.exec("openclaw start")`）
3. 等待健康检查通过（轮询 `pgrep -f openclaw`，最多 30 秒）
4. 切换到主界面，OpenClaw 页显示管理面板

```rust
pub async fn post_install_start(instance: &ClawInstance) -> Result<()> {
    let backend = instance.backend();
    backend.start().await?;
    backend.exec("openclaw start --daemon").await?;

    // 健康检查
    for _ in 0..30 {
        if let Ok(out) = backend.exec("pgrep -f openclaw").await {
            if !out.trim().is_empty() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(anyhow!("OpenClaw 启动超时"))
}
```

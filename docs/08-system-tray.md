# 8. System Tray 规格

## 8.1 设计目标

ClawEnv 安装完成后常驻系统托盘，提供 OpenClaw 实例的实时状态监控与快捷操作，
用户无需打开主窗口即可掌握运行状态、执行常用操作、接收安全告警。

## 8.2 托盘图标状态

| 图标状态 | 颜色/样式 | 含义 |
|---------|----------|------|
| 实心绿色 | `#22c55e` | 至少一个 OpenClaw 实例运行中，无异常 |
| 实心灰色 | `#9ca3af` | 所有实例已停止 |
| 实心红色 | `#ef4444` | 实例异常（崩溃/沙盒不可达） |
| 红色闪烁 | `#ef4444` 脉冲动画 | 高危 CVE 告警（CVSS ≥ 7.0），需立即处理 |
| 橙色提示 | `#f59e0b` | 需要人工介入（CAPTCHA/登录等） |
| 蓝色进度 | `#3b82f6` 带进度弧 | 安装或升级进行中 |

图标基于 ClawEnv logo 的单色轮廓，通过颜色叠加表示状态，确保在浅色/深色系统主题下均清晰可辨。

## 8.3 右键菜单结构

```
┌──────────────────────────────────┐
│  ClawEnv                    v1.0 │
├──────────────────────────────────┤
│  ● default (v2.1.3)    运行中    │  ← 单实例时直接显示
│    ├ 停止                        │
│    ├ 重启                        │
│    └ 查看日志                    │
├──────────────────────────────────┤  ← 多实例时（开发者模式）
│  ● production (v2.1.3)  运行中   │
│  ○ staging (v2.1.4)     已停止   │
│  ▸ 更多实例...                   │
├──────────────────────────────────┤
│  ⚠ 安全更新可用 (v2.1.4)         │  ← 有更新时显示，点击打开更新界面
├──────────────────────────────────┤
│  安装进度: 65%                   │  ← 安装/升级进行中时显示
│  "正在初始化 Alpine 环境..."     │
├──────────────────────────────────┤
│  打开 ClawEnv                    │  ← 打开主窗口
│  设置                            │
│  退出                            │
└──────────────────────────────────┘
```

**菜单规则**：
- 普通用户模式：只显示默认实例，菜单精简
- 开发者模式：列出所有实例���其状态，支持展开子菜单
- 安装/升级进行中时，进度项实时更新（每秒刷新 tooltip 与菜单文本��
- 无更新时不显示更新项，保持菜单干净

## 8.4 安装器状态监控

安装和升级过程中，System Tray 提供全程状态反馈：

```
安装流程中的托盘行为：

1. 用户点击"安装" → 托盘图标切换为蓝色进度状态
2. 各阶段通过 Tauri 事件推送：
   - "正在检测系统环境..."     (5%)
   - "正在准备 WSL2/Lima..."   (20%)
   - "正在下载 Alpine Linux..."(40%)
   - "正在安装 OpenClaw..."    (65%)
   - "正在配置安全凭证..."     (80%)
   - "安装完成"               (100%)
3. 安装完成 → 系统通知弹窗 + 图标切换为绿色
4. 安装失败 → 系统通知弹窗 + 图标切换为红色 + 菜单显示"查看错误日志"
```

**升级流程**：
- 托盘菜单中实例名旁显示 `↑ 升级中...`
- tooltip 显示当前进度百分比
- 升级完成后自动刷新版本号显示

## 8.5 通知策略

| 事件 | 通知方式 | 用户交互 |
|------|---------|---------|
| 安装/升级完成 | 系统通知弹窗（一次） | 点击打开主界面 |
| 安装/升级失败 | 系统通知弹窗 + 托盘变红 | 点击查看日志 |
| 高危 CVE（≥ 7.0） | 系统通知弹窗 + 图标闪烁 | 点击直达更新页 |
| 中危 CVE（4.0–6.9） | 托盘菜单显示更新提示 | 用户主动点击更新 |
| 实例异常崩溃 | 系统通知弹窗 + 托盘变红 | 点击查看日志/重启 |
| 实例自动恢复 | 静默恢复，托盘变绿 | 无需操作 |
| 需要人工操作 | 系统通知弹窗 + 托盘图标切换为橙色 | 点击聚焦到 noVNC 面板 |

## 8.6 Tauri 实现要点

```rust
// tauri/src/tray.rs

use tauri::{
    tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState},
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder, PredefinedMenuItem},
    AppHandle, Manager,
};

/// 托盘图标状态枚举
#[derive(Clone, Copy, PartialEq)]
pub enum TrayStatus {
    Running,           // 绿色 — 实例运行中
    Stopped,           // 灰色 — 所有实例停止
    Error,             // 红色 — 实例异常
    CveAlert,          // 红色闪烁 — 高危 CVE
    Installing(u8),    // 蓝色带进度 — 安装/升级中 (0-100)
}

/// 初始化系统托盘
pub fn setup_tray(app: &AppHandle) -> Result<()> {
    let tray = TrayIconBuilder::with_id("clawenv-tray")
        .icon(load_tray_icon(TrayStatus::Stopped))
        .tooltip("ClawEnv — OpenClaw 已停止")
        .menu(&build_tray_menu(app)?)
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(|tray, event| {
            // 左键单击：打开/聚焦主窗口
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up, ..
            } = event {
                if let Some(win) = tray.app_handle().get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// 根据实例状态动态重建菜单
pub fn refresh_tray(app: &AppHandle, instances: &[ClawInstance], status: TrayStatus) {
    if let Some(tray) = app.tray_by_id("clawenv-tray") {
        tray.set_icon(Some(load_tray_icon(status))).ok();
        tray.set_tooltip(Some(&build_tooltip(instances, status))).ok();
        if let Ok(menu) = build_tray_menu_with_instances(app, instances) {
            tray.set_menu(Some(menu)).ok();
        }
    }
}
```

## 8.7 状态轮询机制

```rust
// core/src/monitor.rs

/// 后台状态监控——定期检查沙盒内 OpenClaw 进程状态
pub struct InstanceMonitor {
    interval: Duration,    // 默认 5 秒
}

impl InstanceMonitor {
    /// 启动监控循环，通过 Tauri 事件通知前端与托盘
    pub async fn start(&self, app: AppHandle, instances: Vec<ClawInstance>) {
        loop {
            for inst in &instances {
                let backend = inst.backend();
                let health = match backend.exec("pgrep -f openclaw").await {
                    Ok(out) if !out.trim().is_empty() => InstanceHealth::Running,
                    Ok(_) => InstanceHealth::Stopped,
                    Err(_) => InstanceHealth::Unreachable,
                };
                app.emit("instance-health", HealthEvent {
                    name: inst.name.clone(),
                    health,
                }).ok();
            }
            tokio::time::sleep(self.interval).await;
        }
    }
}
```

## 8.8 配置扩展

在 `~/.clawenv/config.toml` 中增加托盘相关配置：

```toml
[clawenv.tray]
enabled              = true       # 是否启用系统托盘常驻
start_minimized      = false      # 启动时最小化到托盘（不显示主窗口）
show_notifications   = true       # 是否显示系统通知
monitor_interval_sec = 5          # 实例状态轮询间隔（秒）
```

# 7. 功能规格详述

## 7.1 安装核心流程

```rust
pub async fn install(opts: InstallOptions, progress: &dyn ProgressReporter) -> Result<()> {
    // 1. 平台检测，自动选择对等后端
    let backend = detect_backend().await?;
    progress.report(&format!("检测到平台: {}", backend.name()), 5);

    // 2. 前提条件（WSL2 / limactl / podman）
    backend.ensure_prerequisites(progress).await?;
    progress.report("运行环境就绪", 30);

    // 3. 创建 Alpine 沙盒（含 Node.js 和 OpenClaw 安装）
    backend.create(&SandboxOpts::from(&opts)).await?;
    progress.report("Alpine 沙盒创建完成", 65);

    // 4. API Key 写入系统 Keychain（不写配置文件）
    if let Some(key) = &opts.api_key {
        keychain_store("clawenv", &opts.instance_name, key)?;
        backend.exec("openclaw config set apiKey $(clawenv-keyget)").await?;
    }
    progress.report("配置完成", 80);

    // 5. 注册系统服务（可选，开发者模式）
    if opts.install_daemon {
        install_system_daemon(&backend, &opts.instance_name).await?;
    }

    // 6. 持久化实例配置
    ConfigManager::add_instance(ClawInstance {
        name:         opts.instance_name.clone(),
        claw_type:    ClawType::OpenClaw,
        version:      backend.exec("openclaw --version").await?.trim().to_string(),
        sandbox_type: SandboxType::from_os(),
        created_at:   Utc::now(),
        ..Default::default()
    })?;

    progress.report("安装完成", 100);
    Ok(())
}
```

### 7.1.1 双安装模式

ClawEnv 支持两种安装方式，用户在安装向导 Step 3 中选择：

**模式 A：在线构建（默认）**

从零开始构建沙盒环境：下载 Alpine Linux 基础镜像，在其中逐步安装 Node.js、OpenClaw 及可选组件。

- 优点：体积最小（只装需要的），始终获取最新版本
- 缺点：耗时较长（3-10 分钟），需全程联网，受网络环境影响大
- 适用：网络良好、首次安装、需要自定义组件选择

**模式 B：预构建镜像（推荐）**

下载 ClawEnv 官方预构建的完整镜像文件，一次性导入沙盒。

- 优点：速度快（1-3 分钟），可离线安装（提前下载镜像），环境一致性有保障
- 缺点：镜像体积较大（含浏览器 ~280MB，不含 ~150MB），版本更新需等官方发布
- 适用：网络较差、批量部署、需要离线安装

| | 在线构建 | 预构建镜像 |
|---|---|---|
| 安装速度 | 3-10 分钟 | 1-3 分钟 |
| 网络要求 | 全程联网 | 仅下载镜像时联网（或离线导入） |
| 镜像来源 | Alpine 官方 + npm registry | ClawEnv GitHub Releases |
| 自定义程度 | 高（可选组件） | 低（预设配置） |
| 版本灵活性 | 任意版本 | 跟随官方发布周期 |

**预构建镜像格式**：

| 平台 | 镜像格式 | 导入方式 |
|---|---|---|
| Windows (WSL2) | `.tar.gz`（rootfs tarball） | `wsl --import` |
| macOS (Lima) | `.qcow2`（QEMU 磁盘镜像） | `limactl start --disk` |
| Linux (Podman) | OCI 容器镜像 | `podman load` |

```rust
/// 安装模式
pub enum InstallMode {
    /// 在线构建：下载 Alpine base + 逐步安装
    OnlineBuild,
    /// 预构建镜像：下载并导入完整镜像
    PrebuiltImage {
        /// 镜像 URL（GitHub Releases）或本地文件路径
        source: ImageSource,
    },
}

pub enum ImageSource {
    /// 从 GitHub Releases 下载
    Remote { url: String, checksum_sha256: String },
    /// 从本地文件导入（离线安装）
    Local { path: PathBuf },
}
```

### 7.1.2 代理配置

在网络受限环境中，沙盒内的包管理器（apk）、npm、OpenClaw 以及浏览器均需通过代理访问外网。
ClawEnv 在安装向导中提供代理配置入口，设置后在沙盒内全局生效。

**安装向导中的代理 UI**（Step 2 系统检测之后、Step 3 安装方案之前插入）：

```
┌──────────────────────────────────────────────────┐
│  网络设置（可选）                                  │
│                                                  │
│  □ 使用代理服务器                                 │
│                                                  │
│  ┌─ 勾选后展开 ──────────────────────────────┐   │
│  │  HTTP  代理: [http://proxy.example.com:8080]│   │
│  │  HTTPS 代理: [http://proxy.example.com:8080]│   │
│  │  不走代理:   [localhost,127.0.0.1,.local   ]│   │
│  │                                             │   │
│  │  □ 代理需要认证                              │   │
│  │    用户名: [________]                        │   │
│  │    密码:   [________]                        │   │
│  │                                             │   │
│  │  [测试连接]                                  │   │
│  └─────────────────────────────────────────────┘   │
│                                                  │
│  [上一步]                            [下一步]     │
└──────────────────────────────────────────────────┘
```

**"测试连接"按钮**：尝试通过配置的代理访问 `https://dl-cdn.alpinelinux.org`，
成功显示绿色勾，失败显示具体错误信息。

**沙盒内代理生效方式**：

```rust
/// 将代理配置注入沙盒环境
pub async fn apply_proxy(backend: &dyn SandboxBackend, proxy: &ProxyConfig) -> Result<()> {
    let envs = format!(
        "export http_proxy={hp}\nexport https_proxy={hsp}\nexport no_proxy={np}\n",
        hp = proxy.http_proxy,
        hsp = proxy.https_proxy.as_deref().unwrap_or(&proxy.http_proxy),
        np = proxy.no_proxy.as_deref().unwrap_or("localhost,127.0.0.1"),
    );
    // 写入沙盒的 /etc/profile.d/proxy.sh，对所有进程生效
    backend.exec(&format!("cat > /etc/profile.d/proxy.sh << 'EOF'\n{envs}EOF")).await?;
    // npm 单独配置
    backend.exec(&format!("npm config set proxy {}", proxy.http_proxy)).await?;
    backend.exec(&format!("npm config set https-proxy {}",
        proxy.https_proxy.as_deref().unwrap_or(&proxy.http_proxy))).await?;
    Ok(())
}
```

## 7.2 升级与安全管理

```rust
pub struct VersionInfo {
    pub current:             semver::Version,
    pub latest:              semver::Version,
    pub changelog:           String,
    pub cve_advisories:      Vec<CveAdvisory>,
    pub is_security_release: bool,
}

pub async fn upgrade(instance: &ClawInstance, target: Option<&str>) -> Result<()> {
    // 1. 执行升级（在沙盒内）
    let version = target.unwrap_or("latest");
    instance.backend()
        .exec(&format!("npm update -g openclaw@{version}"))
        .await?;

    // 3. 验证
    let new_ver = instance.backend().exec("openclaw --version").await?;
    ConfigManager::update_version(&instance.name, new_ver.trim())?;

    Ok(())
}

// CVE 通知策略
fn notify_by_severity(cves: &[CveAdvisory]) {
    let max_cvss = cves.iter().map(|c| c.cvss_score).fold(0.0f32, f32::max);
    match max_cvss {
        s if s >= 7.0 => {
            // 高危：立即推送系统通知 + 主界面置顶警告横幅
            push_system_notification(NotificationLevel::Critical, cves);
            set_ui_banner(BannerLevel::Critical, cves);
        },
        s if s >= 4.0 => {
            // 中危：下次启动时提示
            set_pending_notification(NotificationLevel::Warning, cves);
        },
        _ => {
            // 低危：更新日志中列出
            append_to_changelog(cves);
        }
    }
}
```

---

## 7.3 用户模式设计

### 首次启动：模式选择

```
┌──────────────────────────────────────────────────────────────┐
│                  ClawEnv                                     │
│            OpenClaw 安装与管理工具                            │
│                                                              │
│   请选择您的使用方式（可在设置中随时切换）：                   │
│                                                              │
│  ┌───────────────────────────┐  ┌───────────────────────┐   │
│  │                           │  │                       │   │
│  │      普通用户模式          │  │       开发者模式        │   │
│  │                           │  │                       │   │
│  │  - 图形化向导安装          │  │  - 完整 CLI 工具       │   │
│  │  - 自动沙盒（推荐/安全）   │  │  - 多实例管理          │   │
│  │  - 一键升级与安全提醒      │  │  - 原生/沙盒自由选择   │   │
│  │  - 无需技术知识            │  │  - Skill 开发脚手架    │   │
│  │                           │  │  - 快照与回滚          │   │
│  │      [ 选择此模式 ]        │  │    [ 选择此模式 ]      │   │
│  └───────────────────────────┘  └───────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

### 普通用户模式：7 步安装向导

```
Step 1  欢迎        → OpenClaw 简介，需要准备什么
Step 2  系统检测    → 自动检测 OS/架构/已有环境，给出结论
Step 3  网络设置    → 代理配置（可选），测试连接（详见 7.1.2）
Step 4  安装方案    → 选择安装模式（在线构建/预构建镜像，详见 7.1.1），显示对应平台方案
                     （Windows: WSL2 + Alpine，macOS: Lima + Alpine，Linux: Podman + Alpine）
Step 5  API Key    → 输入框 + 格式校验 + 安全提示（存入系统 Keychain）
Step 6  安装进度    → 实时进度条，友好语言（"正在准备 Alpine Linux 环境..."）
Step 7  完成        → 展示如何使用，提供测试按钮
```

### 开发者模式：CLI 命令集

```bash
# 实例管理
clawenv install   [--mode native|sandbox] [--version <ver>] [--name <n>]
clawenv uninstall [--name <n>]
clawenv list                          # 列出所有实例
clawenv create    --name <n>          # 创建新实例
clawenv start|stop|restart [<n>]
clawenv status    [<n>] [--json]
clawenv logs      [<n>] [-f] [--lines 100]

# 升级管理
clawenv upgrade   [<n>] [--version <ver>]
clawenv update check                  # 检查 ClawEnv 自身更新

# 沙盒操作（开发者专属）
clawenv sandbox shell   [<n>]         # 进入沙盒交互式 shell
clawenv sandbox exec    <cmd> [<n>]   # 在沙盒内执行单条命令

# 浏览器集成
clawenv browser start|stop|status [<n>]

# Skill 开发
clawenv skill init    <n>             # 创建 Skill 项目
clawenv skill add     <skill-id>      # 安装 Skill
clawenv skill list    [--installed]
clawenv skill test                    # 本地测试当前目录的 Skill
clawenv skill publish                 # 发布到 Registry

# 诊断
clawenv doctor                        # 诊断当前环境
clawenv config get|set|edit [<key>]
```

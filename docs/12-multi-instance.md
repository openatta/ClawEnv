# 12. 多实例架构设计

## 12.1 概述

ClawEnv 支持两种多实例运行模式：

- **方案 A：独立沙盒** — 每个 OpenClaw 实例运行在独立的 VM/容器中
- **方案 B：共享 VM + Podman** — 一个 VM 内运行多个 Podman 容器

两种模式可以共存，用户通过 `--shared` 参数选择。

## 12.2 方案 A：独立沙盒（默认）

每个实例拥有独立的 VM（macOS/Windows）或容器（Linux）。

```
macOS:                         Linux:
┌────────────────┐            ┌────────────────┐
│ Lima VM: prod  │            │ Podman: prod   │
│ OpenClaw :3000 │            │ OpenClaw :3000 │
└────────────────┘            └────────────────┘
┌────────────────┐            ┌────────────────┐
│ Lima VM: stage │            │ Podman: stage  │
│ OpenClaw :3001 │            │ OpenClaw :3001 │
└────────────────┘            └────────────────┘
```

**优点**：完全隔离、独立快照、故障不扩散
**缺点**：资源占用高（每 VM 512MB+）

**命令**：
```bash
clawenv create --name production
clawenv create --name staging
```

## 12.3 方案 B：共享 VM + Podman 容器

macOS/Windows 上只运行一个 VM，在 VM 内通过 Podman 创建多个容器。
Linux 上直接多个 Podman 容器（与现有方案一致）。

```
macOS/Windows:                   Linux:
┌──────────────────────┐       ┌──────────────────────┐
│ Lima/WSL2 VM         │       │ Host OS              │
│  ┌──────────────┐    │       │  ┌──────────────┐    │
│  │ Podman       │    │       │  │ Podman       │    │
│  │ ├ prod :3000 │    │       │  │ ├ prod :3000 │    │
│  │ ├ stage:3001 │    │       │  │ ├ stage:3001 │    │
│  │ └ dev  :3002 │    │       │  │ └ dev  :3002 │    │
│  └──────────────┘    │       │  └──────────────┘    │
└──────────────────────┘       └──────────────────────┘
总内存: 512MB + 容器增量         总内存: 仅容器
```

**优点**：资源高效、启动快、架构统一
**缺点**：容器级隔离（非 VM 级）、共享内核故障风险

**命令**：
```bash
# 第一个实例创建 VM + Podman
clawenv create --name production

# 后续实例共享同一 VM
clawenv create --name staging --shared production
```

## 12.4 实现设计

### SandboxMode 枚举

```rust
enum SandboxMode {
    /// 独立 VM/容器（方案 A）
    Dedicated(Box<dyn SandboxBackend>),
    /// 共享 VM + Podman 容器（方案 B）
    SharedVm {
        vm: Box<dyn SandboxBackend>,   // Lima/WSL2 宿主 VM
        container: String,              // VM 内的 Podman 容器名
    },
}
```

### VM 内 Podman 管理

```rust
/// 在 VM 内操作 Podman（通过 VM 的 exec 接口）
struct PodmanInVmBackend {
    vm: Arc<dyn SandboxBackend>,        // 外层 VM
    container_name: String,              // VM 内的容器名
    image_tag: String,                   // VM 内的容器镜像
}

impl SandboxBackend for PodmanInVmBackend {
    async fn exec(&self, cmd: &str) -> Result<String> {
        // 通过 VM exec 调用 podman exec
        self.vm.exec(&format!(
            "podman exec {} sh -c '{}'",
            self.container_name, cmd
        )).await
    }
}
```

### 端口分配策略

| 实例 | Gateway Port | 分配规则 |
|------|-------------|---------|
| 第 1 个 | 3000 | 默认 |
| 第 2 个 | 3001 | 自增 |
| 第 N 个 | 3000+N-1 | 范围 3000-3099 |

端口在 `config.toml` 的 `[[instances]]` 中持久化。

### VM 内 Podman 安装

macOS (Lima Alpine):
```sh
apk add podman fuse-overlayfs slirp4netns
```

Windows (WSL2 Alpine):
```sh
apk add podman fuse-overlayfs slirp4netns
```

## 12.5 实施路线

| Phase | 内容 | 状态 |
|-------|------|------|
| Phase 1 | 独立沙盒（当前实现） | ✅ 已完成 |
| Phase 2 | Linux 多 Podman 容器（方案 C） | 🔄 进行中 |
| Phase 3 | macOS/Windows VM 内 Podman | 📋 计划 |
| Phase 4 | `--shared` CLI 选项 + UI | 📋 计划 |

## 12.6 配置格式

```toml
[[instances]]
name = "production"
sandbox_type = "podman-alpine"     # 或 "lima-alpine"
sandbox_mode = "dedicated"          # "dedicated" | "shared"
shared_with = ""                    # 共享模式时指向宿主实例名

[[instances]]
name = "staging"
sandbox_type = "podman-alpine"
sandbox_mode = "shared"
shared_with = "production"          # 共享 production 的 VM
```

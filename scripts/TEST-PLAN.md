# ClawEnv CLI Test Plan — Cross-Platform

## 概述

三个平台各一个独立测试脚本，按阶段分步执行，支持单独跑某个阶段。

```
scripts/
├── test-macos.sh      # macOS 全量测试
├── test-windows.sh    # Windows 全量测试（通过 SSH）
├── test-linux.sh      # Linux 全量测试（本地或 Lima Ubuntu VM）
└── test-cli-full.sh   # 保留：跨平台快速验证（已有）
```

---

## 测试矩阵

| 阶段 | macOS | Windows | Linux |
|------|-------|---------|-------|
| **A. 系统探查** | 本地 | SSH | 本地/VM |
| **B. Native 安装** | ✅ 实施 | ✅ 实施 | ✅ 实施 |
| **C. Native Bundle 导入** | ✅ 实施 | ✅ 实施 | ✅ 实施 |
| **D. 沙盒安装 (Lima)** | ✅ 实施 | — | — |
| **E. 沙盒安装 (WSL2)** | — | ⏸ 暂不实施(虚拟机) | — |
| **F. 沙盒安装 (Podman)** | — | — | ⏸ 暂不实施 |
| **G. Lima 映像导入** | ✅ 实施 | — | — |
| **H. WSL2 映像导入** | — | ⏸ 暂不实施 | — |
| **I. Bridge 测试** | ✅ 实施 | ✅ 实施 | ✅ 实施 |
| **J. 生命周期** | ✅ 实施 | ✅ 实施 | ✅ 实施 |
| **K. 配置/编辑** | ✅ 实施 | ✅ 实施 | ✅ 实施 |
| **L. 清理** | ✅ 实施 | ✅ 实施 | ✅ 实施 |

---

## 阶段详细设计

### A. 系统探查（3平台通用）

```bash
clawenv doctor                    # OS/内存/磁盘/沙盒后端
clawenv system-check              # 详细检查项（含 Proxy 检测）
clawenv claw-types                # 注册表可用产品
clawenv list                      # 已有实例
clawenv config show               # 当前配置
clawenv sandbox list              # 本机 VM/容器
clawenv sandbox info              # 沙盒磁盘
```

### B. Native 安装（3平台通用）

```bash
# 分步安装
clawenv install --mode native --name test-native --step prereq
clawenv install --mode native --name test-native --step create
clawenv install --mode native --name test-native --step claw    # npm install
clawenv install --mode native --name test-native --step config --port 3200
clawenv install --mode native --name test-native --step gateway --port 3200

# 验证
clawenv status test-native
clawenv exec "echo ok" test-native
curl -s http://127.0.0.1:3200/ | head -1    # gateway HTTP 响应
```

### C. Native Bundle 导入（3平台通用）

```bash
# 前置：生成 bundle
bash tools/package-native.sh openclaw latest ./test-bundle

# 导入测试
clawenv install --mode native --name test-bundle \
  --image ./test-bundle/clawenv-native-*.tar.gz --port 3300

# 验证
clawenv status test-bundle
clawenv exec "openclaw --version" test-bundle
```

**注意**: 生成 bundle 需要联网（npm install），耗时 5-10 分钟。可以预先生成一次，多次导入测试。

### D. 沙盒安装 — Lima (macOS only)

```bash
# 前置检查
clawenv install --mode sandbox --name test-sandbox --step prereq    # 检查 Lima

# 创建 VM（7-10 分钟）
clawenv install --mode sandbox --name test-sandbox --step create

# 安装 Claw（5-10 分钟）
clawenv install --mode sandbox --name test-sandbox --step claw

# 配置 + 启动
clawenv install --mode sandbox --name test-sandbox --step config --port 3400
clawenv install --mode sandbox --name test-sandbox --step gateway --port 3400

# 验证
clawenv status test-sandbox
clawenv exec "node --version" test-sandbox
clawenv exec "openclaw --version" test-sandbox
clawenv sandbox shell test-sandbox <<< "echo hello && exit"  # 交互式 shell

# 生命周期
clawenv stop test-sandbox
clawenv start test-sandbox
clawenv restart test-sandbox
```

**预计耗时**: 15-25 分钟（含 VM 创建 + npm install）

### E. 沙盒安装 — WSL2 (Windows, 暂不实施)

与 D 类似，但使用 `wsl --import` 代替 Lima。需要非虚拟机的 Windows 物理机。

### F. 沙盒安装 — Podman (Linux, 暂不实施)

需要 Linux 环境。可以通过 Lima Ubuntu VM 在 macOS 上模拟，但操作复杂，暂缓。

### G. Lima 映像导入 (macOS only)

```bash
# 前置：导出现有 sandbox 实例
clawenv export test-sandbox --output ./test-export

# 导入到新实例
clawenv import ./test-export/*.tar.gz --name test-import

# 验证
clawenv start test-import
clawenv exec "openclaw --version" test-import
```

### H. WSL2 映像导入 (Windows, 暂不实施)

### I. Bridge 测试（3平台通用，依赖 native 实例存在）

```bash
# 前置：bridge 必须在 config 中 enabled
clawenv config set bridge.enabled true

# 启动 bridge（由 Tauri GUI 自动启动，CLI 可手动测试）
# 实际上 bridge 是 Tauri main.rs 启动的，CLI 无直接启动命令
# 测试方式：用 curl 直接探测

# 健康检查
curl -s http://127.0.0.1:3100/api/health

# 权限查询
curl -s http://127.0.0.1:3100/api/permissions

# 文件读取（需要权限允许）
curl -s -X POST http://127.0.0.1:3100/api/file/read \
  -H "Content-Type: application/json" \
  -d '{"path":"~/.clawenv/config.toml"}'
```

**注意**: Bridge 目前只由 Tauri GUI 启动。CLI 测试需要：
1. 新增 CLI 命令 `clawenv bridge start` 和 `clawenv bridge test`
2. 或在测试脚本中直接启动 bridge 进程

**建议**: 新增 `clawenv bridge test` 命令，检查 bridge 是否可达。

### J. 生命周期（3平台通用）

```bash
clawenv start <instance>
clawenv stop <instance>
clawenv restart <instance>
clawenv status <instance>
clawenv logs <instance>
clawenv exec "echo test" <instance>
clawenv update-check <instance>
```

### K. 配置/编辑（3平台通用）

```bash
# Config 往返
clawenv config set language en-US
clawenv config show | grep en-US
clawenv config set language zh-CN

# 端口编辑
clawenv edit <instance> --gateway-port 3201
clawenv status <instance> | grep 3201
clawenv edit <instance> --gateway-port 3200

# 端口冲突
clawenv install --mode native --name conflict-test --port 3200 --step config  # 应该报错

# Proxy 测试
clawenv config proxy-test
```

### L. 清理（3平台通用）

```bash
clawenv uninstall --name test-native
clawenv uninstall --name test-bundle
clawenv uninstall --name test-sandbox    # macOS only
clawenv uninstall --name test-import     # macOS only
clawenv list  # 应该为空
```

---

## 实施任务分解

### 任务 1: 新增 `clawenv bridge test` CLI 命令
- 检查 bridge 是否在运行（HTTP GET /api/health）
- 返回 JSON 结果（status, version, port）
- 如果 bridge 未运行，返回明确错误

### 任务 2: 重构测试脚本 — test-macos.sh
- 包含: A + B + C + D + G + I + J + K + L
- 支持 `--phase A|B|C|D|G|I|J|K|L` 单独执行某阶段
- 支持 `--skip-sandbox` 跳过耗时的沙盒阶段
- 预计全量运行: 30-40 分钟

### 任务 3: 重构测试脚本 — test-windows.sh
- 通过 SSH 执行所有命令
- 包含: A + B + C + I + J + K + L
- 支持 `--phase` 选择
- 预计全量运行: 15-20 分钟
- WSL2 沙盒(D/E)和映像导入(H)标记为 SKIP

### 任务 4: 重构测试脚本 — test-linux.sh
- 包含: A + B + C + I + J + K + L
- 支持 `--phase` 选择
- Podman 沙盒(F)标记为 SKIP
- 可在 Lima Ubuntu VM 或 CI 容器中运行

### 任务 5: 生成 native bundle 脚本增强
- package-native.sh 需要验证可用
- 测试脚本调用它生成 bundle 然后导入

### 任务 6: 修复发现的问题
- Bridge CLI 命令缺失（新增 bridge test）
- 导入/导出流程可能的 bug
- 沙盒完整生命周期中的平台差异

---

## 执行顺序

```
第一步: 任务 1 (bridge test 命令)
第二步: 任务 2 (test-macos.sh) — 在本地跑通全量
第三步: 任务 3 (test-windows.sh) — 通过 SSH 跑通
第四步: 任务 4 (test-linux.sh) — 框架搭好，标记暂不实施的阶段
第五步: 任务 5+6 — 补充 bundle 生成 + 修复发现的问题
```

## 预计工作量

| 任务 | 工作量 | 测试运行时间 |
|------|--------|-------------|
| bridge test 命令 | 小（30min） | — |
| test-macos.sh | 中（含 sandbox 15-25min 等待） | ~40min 全量 |
| test-windows.sh | 中（SSH + npm install 等待） | ~20min 全量 |
| test-linux.sh | 小（框架+SKIP） | ~15min 可用阶段 |
| bundle + 修复 | 视发现问题 | — |

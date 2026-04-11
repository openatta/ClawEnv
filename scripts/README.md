# ClawEnv — 测试与打包脚本

## 目录

| 脚本 | 用途 |
|------|------|
| `test-claw-runner.sh` | 自动化测试调度器（入口） |
| `test-claw-lifecycle.sh` | 单个 claw 完整生命周期测试 |
| `test-claw-install.sh` | 单个 claw 轻量安装测试 |
| `package-alpine.sh` | 沙盒映像打包 |
| `package-native.sh` | Native 离线安装包打包 |

---

## 测试框架

### 三层测试架构

| 层级 | 命令 | 耗时 | 说明 |
|------|------|------|------|
| L1 纯函数 | `cargo test -p clawenv-core claw::tests` | <1s | ClawDescriptor 命令格式化 + Registry 操作 |
| L2 Mock 流程 | `cargo test -p clawenv-core claw::flow_tests` | <1s | 用 MockBackend 验证 install/upgrade/lifecycle 命令序列 |
| L3 真实沙盒 | `bash scripts/test-claw-runner.sh` | 5-50min | 在真实 Lima/Podman 沙盒中安装、启动、验证 |

### L3 测试使用指南

#### 1. 快速测试（仅安装+版本验证，不测 gateway）

```bash
# 测试所有内置 claw（串行）
bash scripts/test-claw-install.sh --all

# 测试单个 claw
bash scripts/test-claw-install.sh openclaw

# 通过 runner 并行快速测试
bash scripts/test-claw-runner.sh --quick --parallel 4
```

#### 2. 单个 claw 完整生命周期

```bash
# 默认 5 分钟超时
bash scripts/test-claw-lifecycle.sh openclaw

# 自定义输出目录和超时
bash scripts/test-claw-lifecycle.sh zeroclaw ./my-results 600
```

7 个步骤：创建沙盒 → 安装 → 版本验证 → 启动 gateway → HTTP 健康检查 → API Key 配置 → 停止 → 销毁

#### 3. 并行测试指定 claw

```bash
# 并行 2 个
bash scripts/test-claw-runner.sh --claws "openclaw zeroclaw autoclaw" --parallel 2

# 并行 4 个，超时 10 分钟
bash scripts/test-claw-runner.sh --claws "openclaw zeroclaw" --parallel 4 --timeout 600
```

#### 4. 完整测试所有 claw

```bash
# 默认：并行 2，超时 300s，失败重试 1 次
bash scripts/test-claw-runner.sh

# 高配：并行 4，超时 600s，重试 2 次
bash scripts/test-claw-runner.sh --parallel 4 --timeout 600 --retry 2
```

### 测试输出

```
test-results/
├── result-openclaw.toml     # 每个 claw 的详细结果
├── result-zeroclaw.toml
├── log-openclaw.txt         # 完整日志
├── summary-20260411.toml    # 聚合报告
```

#### 结果 TOML 格式

```toml
[result]
claw_id = "openclaw"
status = "pass"              # pass / fail / timeout / skip
install_duration_sec = 47

[[steps]]
name = "install"
status = "pass"
duration_sec = 47

[[steps]]
name = "gateway_start"
status = "pass"
duration_sec = 8
detail = "HTTP 200"
```

### 前置条件

- macOS: 需要 `lima` (`brew install lima`)
- Linux: 需要 `podman` (`sudo apt install podman`)
- 网络: 需要访问 npm registry（或配置镜像源）
- 内存: 每个并行沙盒约 512MB

### CI 集成

```bash
# GitHub Actions / CI 中使用
bash scripts/test-claw-runner.sh --quick --parallel 2
echo "Exit code: $?"  # 0=全部通过, 1=有失败
```

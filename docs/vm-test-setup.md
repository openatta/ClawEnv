# ClawEnv 跨平台测试环境搭建

在 VM 上搭建最小编译环境，用于测试 ClawEnv 的 Ubuntu 和 Windows 版本。

## 前提

- 宿主机已有 Git 仓库，VM 通过 Git clone/pull 同步代码
- VM 能访问互联网（下载依赖）

---

## Ubuntu 22.04+

### 1. 系统依赖（一次性）

```bash
# 更新系统
sudo apt update && sudo apt upgrade -y

# Tauri 编译依赖
sudo apt install -y \
  build-essential \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  curl wget git \
  pkg-config \
  file

# Node.js 22 (Tauri 前端构建)
curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
sudo apt install -y nodejs

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env

# 验证
node --version    # v22.x
npm --version     # 10.x+
rustc --version   # 1.80+
cargo --version
```

### 2. 克隆并编译

```bash
# 克隆代码（替换为你的仓库地址）
git clone <your-repo-url> ClawEnv
cd ClawEnv

# 安装前端依赖
npm install

# 编译（Release 模式，约 3-5 分钟）
cargo tauri build

# 产物位置
ls target/release/bundle/deb/    # .deb 安装包
ls target/release/bundle/appimage/  # AppImage
ls target/release/clawenv-tauri  # 二进制
```

### 3. 运行并收集日志

```bash
# 运行（带详细日志）
RUST_LOG=debug ./target/release/clawenv-tauri 2>&1 | tee /tmp/clawenv-debug.log

# 或安装 deb 后运行
sudo dpkg -i target/release/bundle/deb/clawenv_*.deb
RUST_LOG=debug clawenv-tauri 2>&1 | tee /tmp/clawenv-debug.log
```

### 4. 日志回传

```bash
# 方法1：直接复制日志文件到共享目录
cp /tmp/clawenv-debug.log /path/to/shared/

# 方法2：通过 Git
cp /tmp/clawenv-debug.log docs/logs/ubuntu-test.log
git add docs/logs/ && git commit -m "Ubuntu test log" && git push
```

### 5. 代码更新后重新测试

```bash
cd ClawEnv
git pull
npm install          # 如果前端依赖变了
cargo tauri build    # 重新编译
```

---

## Windows 10 2004+

### 1. 系统依赖（一次性）

**步骤 A：Visual Studio Build Tools**

下载并安装：https://visualstudio.microsoft.com/visual-cpp-build-tools/

安装时勾选：
- "C++ 桌面开发" 工作负载
- 确保包含 Windows 10/11 SDK

**步骤 B：通过 PowerShell 安装其余工具**（管理员模式）

```powershell
# 安装 Rust
winget install Rustlang.Rustup
# 重启终端后验证
rustup default stable
rustc --version

# 安装 Node.js 22
winget install OpenJS.NodeJS.LTS
# 重启终端后验证
node --version
npm --version

# 安装 Git
winget install Git.Git

# 安装 WebView2 运行时（Windows 11 已预装，Win10 可能需要）
# 下载：https://developer.microsoft.com/en-us/microsoft-edge/webview2/
```

### 2. 克隆并编译

```powershell
# 克隆代码
git clone <your-repo-url> ClawEnv
cd ClawEnv

# 安装前端依赖
npm install

# 编译（Release 模式，约 5-10 分钟）
cargo tauri build

# 产物位置
dir target\release\bundle\msi\    # .msi 安装包
dir target\release\bundle\nsis\   # NSIS 安装包
dir target\release\clawenv-tauri.exe
```

### 3. 运行并收集日志

```powershell
# 运行（带详细日志）
$env:RUST_LOG="debug"
.\target\release\clawenv-tauri.exe 2>&1 | Tee-Object -FilePath C:\clawenv-debug.log

# 或安装 msi 后运行
msiexec /i target\release\bundle\msi\ClawEnv_*.msi
$env:RUST_LOG="debug"
& "C:\Program Files\ClawEnv\clawenv-tauri.exe" 2>&1 | Tee-Object -FilePath C:\clawenv-debug.log
```

### 4. 日志回传

```powershell
# 方法1：复制到共享目录
copy C:\clawenv-debug.log Z:\shared\

# 方法2：通过 Git
copy C:\clawenv-debug.log docs\logs\windows-test.log
git add docs\logs\ && git commit -m "Windows test log" && git push
```

### 5. WSL2 安装验证

ClawEnv 在 Windows 上使用 WSL2 作为沙盒后端。测试 WSL2 相关功能前确认：

```powershell
# 检查 WSL2 状态
wsl --status

# 如果未安装
wsl --install
# 需要重启电脑！

# 重启后验证
wsl --version
```

---

## 测试检查清单

在 VM 上运行 ClawEnv 后，依次验证：

- [ ] 程序启动，主窗口显示
- [ ] System Tray 图标出现
- [ ] 安装向导流程（选 Sandbox 或 Native）
- [ ] Sandbox 安装：VM/WSL2/Podman 创建成功
- [ ] Native 安装：Node.js 检测/安装 + OpenClaw 安装
- [ ] OpenClaw gateway 启动
- [ ] Terminal (ttyd) 连接
- [ ] 实例 Start/Stop/Restart
- [ ] 关闭窗口 → Tray 驻留
- [ ] Tray 菜单操作

## 常见问题

**Ubuntu: `libwebkit2gtk-4.1-dev` 找不到**
→ 确认 Ubuntu 版本 ≥ 22.04。20.04 只有 4.0 版本，不兼容。

**Windows: `cargo tauri build` 报找不到 link.exe**
→ 确认 Visual Studio Build Tools 安装了 "C++ 桌面开发" 工作负载。

**Windows: WebView2 相关错误**
→ 下载安装 WebView2 Evergreen Runtime。

**编译很慢**
→ 首次编译 3-10 分钟正常。后续增量编译快很多。可以用 `cargo tauri build --debug` 加速（不优化）。

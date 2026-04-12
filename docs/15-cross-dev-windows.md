# Windows ARM64 交叉开发指南

> 从 macOS 通过 SSH 远程控制 Windows ARM64 (UTM) 虚拟机进行构建和测试

## 环境信息

| 项目 | 值 |
|------|---|
| Windows VM | UTM on Apple Silicon, Windows 11 ARM64 |
| SSH | OpenSSH Server (Windows 内置) |
| 连接信息 | 存储在项目根目录 `.env` 文件中（git ignored） |

### `.env` 格式
```
WIN_HOST=192.168.64.7
WIN_USER=clawenv
WIN_PASS=clawenv
```

## 已安装的开发工具

| 工具 | 版本 | 安装方式 |
|------|------|---------|
| Rust | 1.94.1 (aarch64-pc-windows-msvc) | rustup-init.exe 直接下载 |
| Git | 2.49.0 (arm64) | 直接下载 exe |
| Node.js | v22.16.0 (arm64) | 直接下载 msi |
| VS Build Tools 2022 | MSVC 14.44.35207 + ARM64 libs | VS Installer (本地安装) |
| LLVM/Clang | 19.1.7 (woa64) | 直接下载 exe，本地 `/S` 安装 |

### 关键环境变量（已通过 `setx /m` 持久化）

```
LIB = C:\...\MSVC\14.44.35207\lib\arm64;C:\...\Windows Kits\10\Lib\10.0.26100.0\ucrt\arm64;...\um\arm64
INCLUDE = C:\...\MSVC\14.44.35207\include;C:\...\Windows Kits\10\Include\10.0.26100.0\ucrt;...\um;...\shared
PATH += C:\...\MSVC\14.44.35207\bin\Hostx64\x64;C:\Program Files\LLVM\bin
```

**注意**：`setx /m PATH` 设置的系统 PATH 在**新 SSH session** 中生效，但当前 session 不会更新。`win-remote.sh` 脚本通过 `set PATH=` 前缀解决此问题。

## 日常使用

### 快捷脚本

项目提供了 `scripts/win-remote.sh`，封装了所有远程操作：

```bash
# 拉取最新代码
bash scripts/win-remote.sh pull

# 运行测试
bash scripts/win-remote.sh test

# 编译检查
bash scripts/win-remote.sh check

# 安装 npm 依赖
bash scripts/win-remote.sh npm install

# 完整构建 Tauri
bash scripts/win-remote.sh build

# 启动开发模式（GUI 在 Windows 端显示）
bash scripts/win-remote.sh dev

# 打开交互式 SSH
bash scripts/win-remote.sh shell

# 运行任意命令
bash scripts/win-remote.sh run "dir C:\Users\clawenv\Desktop"
```

### 开发循环

```
1. 本机 (macOS) 编辑代码
2. git push
3. bash scripts/win-remote.sh pull
4. bash scripts/win-remote.sh test       ← 自动测试
5. bash scripts/win-remote.sh dev        ← 启动 GUI（Windows 上看）
6. 用户在 Windows 上看 GUI，反馈问题
7. 本机修改代码 → 回到步骤 2
```

### 手动 SSH

```bash
# 免密登��
ssh clawenv@192.168.64.7

# 运行带 MSVC/LLVM 环境的命令
ssh clawenv@192.168.64.7 "set PATH=%PATH%;C:\Program Files\LLVM\bin&& cd C:\Users\clawenv\Desktop\ClawEnv && cargo test -p clawenv-core"
```

## 从零搭建环境

如果虚拟机重建，按以下顺序操作：

### 1. Windows 上开启 SSH
```powershell
# PowerShell (管理员)
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Start-Service sshd
Set-Service -Name sshd -StartupType Automatic
```

### 2. macOS 上部署 SSH Key
```bash
PUB_KEY=$(cat ~/.ssh/id_ed25519.pub)
sshpass -p "clawenv" ssh -o StrictHostKeyChecking=no clawenv@<IP> \
  "echo $PUB_KEY > C:\ProgramData\ssh\administrators_authorized_keys && \
   icacls C:\ProgramData\ssh\administrators_authorized_keys /inheritance:r /grant SYSTEM:(F) /grant *S-1-5-32-544:(F)"
```

### 3. macOS 上远程安装开发工具
```bash
# Git
ssh clawenv@<IP> "curl.exe -L -o %TEMP%\git.exe https://github.com/git-for-windows/git/releases/download/v2.49.0.windows.1/Git-2.49.0-arm64.exe && %TEMP%\git.exe /VERYSILENT /NORESTART"

# Node.js
ssh clawenv@<IP> "curl.exe -L -o %TEMP%\node.msi https://nodejs.org/dist/v22.16.0/node-v22.16.0-arm64.msi && msiexec /i %TEMP%\node.msi /qn /norestart"

# Rust
ssh clawenv@<IP> "curl.exe -L -o %TEMP%\rustup-init.exe https://static.rust-lang.org/rustup/dist/aarch64-pc-windows-msvc/rustup-init.exe && %TEMP%\rustup-init.exe -y --default-toolchain stable"
```

### 4. Windows 本地安装（无法通过 SSH）
以下两项需要在 Windows 桌面上操作（需要 UAC 弹窗或交互式安装器）：

**VS Build Tools 2022：**
```powershell
# 下载
curl.exe -L -o $env:TEMP\vs_buildtools.exe https://aka.ms/vs/17/release/vs_BuildTools.exe
# 安装（含 ARM64 C++ + Clang tools）
& "$env:TEMP\vs_buildtools.exe" --wait --passive --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.VC.Tools.ARM64 --add Microsoft.VisualStudio.Component.VC.Llvm.Clang --add Microsoft.VisualStudio.Component.VC.Llvm.ClangToolset --includeRecommended
```

**LLVM/Clang（完整编译器）：**
```powershell
curl.exe -L -o $env:TEMP\llvm.exe https://github.com/llvm/llvm-project/releases/download/llvmorg-19.1.7/LLVM-19.1.7-woa64.exe
& "$env:TEMP\llvm.exe" /S
```

### 5. macOS 上远程设置环境变量
```bash
# 确认 MSVC 版本号
ssh clawenv@<IP> 'dir "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC"'
# 记下版本号 (如 14.44.35207)，然后设置：

MSVC="C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207"
SDK="C:\Program Files (x86)\Windows Kits\10"

ssh clawenv@<IP> "setx /m LIB \"$MSVC\lib\arm64;$SDK\Lib\10.0.26100.0\ucrt\arm64;$SDK\Lib\10.0.26100.0\um\arm64\""
ssh clawenv@<IP> "setx /m INCLUDE \"$MSVC\include;$SDK\Include\10.0.26100.0\ucrt;$SDK\Include\10.0.26100.0\um;$SDK\Include\10.0.26100.0\shared\""
ssh clawenv@<IP> "setx /m PATH \"%PATH%;$MSVC\bin\Hostx64\x64;C:\Program Files\LLVM\bin\""
```

### 6. Clone 并测试
```bash
ssh clawenv@<IP> '"C:\Program Files\Git\cmd\git.exe" clone https://github.com/openatta/ClawEnv.git C:\Users\clawenv\Desktop\ClawEnv'
bash scripts/win-remote.sh test
```

## 已知问题

| 问题 | 原因 | 解决方案 |
|------|------|---------|
| `setx` 设置的 PATH 在当前 SSH session 不生效 | Windows 环境变量只对新进程生效 | `win-remote.sh` 用 `set PATH=` 前缀 |
| winget 通过 SSH 无法使用 | 需要交互式登录会话 | 用 `curl.exe` 直接下载安装 |
| NSIS 安装器 `/S` 通过 SSH 有时静默失败 | 缺少 UAC 弹窗 | VS Build Tools 和 LLVM 需本地安装 |
| `ring` crate 需要 clang | ARM64 Windows 上 MSVC 的 MASM 不支持 | 安装 LLVM 19.x ARM64 版 |
| MSVC ARM64 lib 在 14.50 版本不完整 | 只有 clang_rt 库 | 使用 14.44 版本的 lib |

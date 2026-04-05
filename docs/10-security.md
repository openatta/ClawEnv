# 10. 安全模型

**凭证安全**：API Key 一律存入系统 Keychain（Windows Credential Manager / macOS Keychain /
Linux Secret Service），`config.toml` 和日志中均不出现明文。

**沙盒隔离原则**：
- 只挂载 `~/.clawenv/workspaces/<instance>/`，不挂载主目录
- Podman 使用 `--userns=keep-id` rootless 模式
- 网络端口只绑定 `127.0.0.1`，不暴露外网
- Lima/WSL2 主目录挂载为只读

**CVE 响应**：CVSS ≥ 7.0 立即推送通知并置顶警告横幅；4.0–6.9 下次启动提示；< 4.0 写入更新日志。

**快照策略**：每次升级前自动创建快照，保留最近 5 个，超出自动删除最旧的。

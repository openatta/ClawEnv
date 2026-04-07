//! Bridge Client — calls ClawEnv Bridge HTTP API on the host machine.

use anyhow::Result;
use serde::Deserialize;

pub struct BridgeClient {
    base_url: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct ExecResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

impl BridgeClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Dispatch a tool call to the appropriate Bridge API endpoint
    pub async fn call_tool(&self, name: &str, args: &serde_json::Value) -> Result<String> {
        match name {
            "file_read" => self.file_read(args).await,
            "file_write" => self.file_write(args).await,
            "file_list" => self.file_list(args).await,
            "exec" => self.exec(args).await,
            "browser_open" => self.browser_open(args).await,
            _ => anyhow::bail!("Unknown tool: {name}"),
        }
    }

    async fn file_read(&self, args: &serde_json::Value) -> Result<String> {
        let path = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'path' argument required"))?;

        let resp = self.client
            .post(format!("{}/api/file/read", self.base_url))
            .json(&serde_json::json!({ "path": path }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Bridge error: {text}");
        }

        let body: serde_json::Value = resp.json().await?;
        Ok(body.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string())
    }

    async fn file_write(&self, args: &serde_json::Value) -> Result<String> {
        let path = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'path' argument required"))?;
        let content = args.get("content").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'content' argument required"))?;

        let resp = self.client
            .post(format!("{}/api/file/write", self.base_url))
            .json(&serde_json::json!({ "path": path, "content": content }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Bridge error: {text}");
        }

        Ok(format!("Written {} bytes to {}", content.len(), path))
    }

    async fn file_list(&self, args: &serde_json::Value) -> Result<String> {
        let path = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'path' argument required"))?;

        let resp = self.client
            .post(format!("{}/api/file/list", self.base_url))
            .json(&serde_json::json!({ "path": path }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Bridge error: {text}");
        }

        let body: serde_json::Value = resp.json().await?;
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn exec(&self, args: &serde_json::Value) -> Result<String> {
        let command = args.get("command").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'command' argument required"))?;
        let cmd_args: Vec<String> = args.get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let resp = self.client
            .post(format!("{}/api/exec", self.base_url))
            .json(&serde_json::json!({ "command": command, "args": cmd_args }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Bridge error: {text}");
        }

        let exec_resp: ExecResponse = resp.json().await?;
        let mut output = exec_resp.stdout;
        if !exec_resp.stderr.is_empty() {
            output.push_str("\n[stderr] ");
            output.push_str(&exec_resp.stderr);
        }
        if exec_resp.exit_code != 0 {
            output.push_str(&format!("\n[exit code: {}]", exec_resp.exit_code));
        }
        Ok(output)
    }

    async fn browser_open(&self, args: &serde_json::Value) -> Result<String> {
        let url = args.get("url").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("'url' argument required"))?;

        // Use exec to open browser on host
        let resp = self.client
            .post(format!("{}/api/exec", self.base_url))
            .json(&serde_json::json!({
                "command": "open",
                "args": [url]
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Bridge error: {text}");
        }

        Ok(format!("Opened {} in host browser", url))
    }
}

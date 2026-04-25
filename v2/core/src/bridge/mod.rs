//! In-VM helpers for the AttaRun bridge: read gateway auth tokens
//! from the claw's config file inside a running sandbox.
//!
//! Lifted from v1 `tauri/src/ipc/bridge.rs::get_gateway_token` (P1-d).
//! The IPC layer in v1 mixed config-loading + backend-instantiation +
//! exec; v2 splits those — caller passes the backend in, this fn just
//! does the in-VM read.

use std::sync::Arc;

use crate::common::OpsError;
use crate::sandbox_backend::SandboxBackend;

/// Read the gateway auth token from inside the sandbox.
///
/// `claw_id` is the identifier ("openclaw", "hermes", ...). The
/// matching config file lives at `~/.<claw_id>/<claw_id>.json` for
/// the user that ran the claw — could be root, clawenv, or any other
/// home dir. We use Node.js to parse JSON reliably (instead of
/// grep/sed) and to scan multiple candidate paths.
///
/// Supports two on-disk schemas:
/// - Old: `{"token": "..."}` at top level
/// - New: `{"gateway": {"auth": {"token": "..."}}}`
///
/// Errors when no token is found in any candidate (common cause: the
/// instance hasn't started yet so no config has been written).
pub async fn read_gateway_token(
    backend: &Arc<dyn SandboxBackend>,
    claw_id: &str,
) -> Result<String, OpsError> {
    if claw_id.is_empty() {
        return Err(OpsError::parse("claw_id cannot be empty"));
    }
    // Defense in depth: claw_id ends up unquoted inside JS string
    // literals; restrict to a conservative charset to keep the script
    // a constant-shape program with one varying word.
    if !claw_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(OpsError::parse(format!(
            "claw_id has unsafe characters: {claw_id}"
        )));
    }

    // node -e <script> reads each candidate path, parses JSON, prints
    // the first non-empty token to stdout. Scans /home/<user>/.<id>/<id>.json
    // for every user dir AND $HOME/.<id>/<id>.json. Verbatim from v1
    // (whitespace tightened, comments stripped — the node syntax is
    // delicate).
    let script = format!(
        r#"node -e "
const fs = require('fs'), path = require('path'), id = '{id}';
let homes = [];
try {{ homes = fs.readdirSync('/home').map(u => '/home/'+u+'/.'+id+'/'+id+'.json'); }} catch {{}}
const home = process.env.HOME || process.env.USERPROFILE || '~';
const candidates = [path.join(home, '.'+id, id+'.json'), ...homes.filter(f => {{ try {{ return fs.existsSync(f); }} catch {{ return false; }} }})];
for (const f of candidates) {{
  try {{
    const j = JSON.parse(fs.readFileSync(f,'utf8'));
    const t = (j.gateway && j.gateway.auth && j.gateway.auth.token) || j.token || '';
    if (t) {{ process.stdout.write(t); process.exit(0); }}
  }} catch {{}}
}}
""#,
        id = claw_id
    );

    let stdout = backend
        .exec_argv(&["sh", "-c", &script])
        .await
        .map_err(OpsError::Other)?;
    let token = stdout.trim().to_string();
    if token.is_empty() {
        return Err(OpsError::not_found(format!(
            "gateway token for `{claw_id}` (is the instance running and configured?)"
        )));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_ops::testing::MockBackend;

    fn arc_mock(stdout: &str) -> Arc<dyn SandboxBackend> {
        Arc::new(MockBackend::new("fake").with_stdout(stdout))
    }

    #[tokio::test]
    async fn happy_path_returns_token() {
        let backend = arc_mock("abc-token-123\n");
        let token = read_gateway_token(&backend, "openclaw").await.unwrap();
        assert_eq!(token, "abc-token-123");
    }

    #[tokio::test]
    async fn empty_stdout_yields_not_found() {
        let backend = arc_mock("");
        let err = read_gateway_token(&backend, "openclaw").await.unwrap_err();
        assert!(matches!(err, OpsError::NotFound { .. }));
    }

    #[tokio::test]
    async fn rejects_empty_claw_id() {
        let backend = arc_mock("");
        let err = read_gateway_token(&backend, "").await.unwrap_err();
        assert!(matches!(err, OpsError::Parse(_)));
    }

    #[tokio::test]
    async fn rejects_unsafe_claw_id() {
        // Quote / semicolon / shell metas would all be defended-in-depth
        // by the JS string literal, but our defensive sanity check
        // catches them earlier.
        let backend = arc_mock("");
        for bad in [r#"foo"; rm -rf /""#, "foo bar", "foo$bar", "foo`bar"] {
            let err = read_gateway_token(&backend, bad).await.unwrap_err();
            assert!(
                matches!(err, OpsError::Parse(_)),
                "expected Parse for {bad}"
            );
        }
    }

    #[tokio::test]
    async fn whitespace_around_token_is_trimmed() {
        let backend = arc_mock("   xyz   \n  ");
        let token = read_gateway_token(&backend, "hermes").await.unwrap();
        assert_eq!(token, "xyz");
    }
}

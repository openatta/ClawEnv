//! Mock `ExecutionContext` for unit tests.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;

use super::{ContextKind, ExecError, ExecutionContext};

pub struct MockContext {
    pub id_str: String,
    pub canned_stdout: Mutex<String>,
    pub exec_log: Mutex<Vec<String>>,
    pub fail_with: Mutex<Option<ExecErrorFactory>>,
    pub alive: bool,
}

pub type ExecErrorFactory = Box<dyn Fn() -> ExecError + Send + Sync>;

impl MockContext {
    pub fn new(id: &str) -> Self {
        Self {
            id_str: id.into(),
            canned_stdout: Mutex::new(String::new()),
            exec_log: Mutex::new(Vec::new()),
            fail_with: Mutex::new(None),
            alive: true,
        }
    }

    pub fn with_stdout(self, s: impl Into<String>) -> Self {
        *self.canned_stdout.lock().unwrap() = s.into();
        self
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = alive;
        self
    }

    pub fn fail(self, factory: ExecErrorFactory) -> Self {
        *self.fail_with.lock().unwrap() = Some(factory);
        self
    }

    pub fn last_argv(&self) -> Option<String> {
        self.exec_log.lock().unwrap().last().cloned()
    }
}

#[async_trait]
impl ExecutionContext for MockContext {
    fn id(&self) -> String {
        self.id_str.clone()
    }

    fn kind(&self) -> ContextKind {
        ContextKind::Native { prefix: PathBuf::from("/mock") }
    }

    async fn exec(&self, argv: &[&str]) -> Result<String, ExecError> {
        self.exec_log.lock().unwrap().push(argv.join(" "));
        if let Some(f) = self.fail_with.lock().unwrap().as_ref() {
            return Err(f());
        }
        Ok(self.canned_stdout.lock().unwrap().clone())
    }

    async fn is_alive(&self) -> bool {
        self.alive
    }

    fn resolve_to_host(&self, p: &Path) -> Option<PathBuf> {
        Some(p.to_path_buf())
    }
}

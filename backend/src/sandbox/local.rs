//! Local sandbox: subprocesses on the host, confined to the run directory by convention only.
//!
//! There is NO isolation here — it exists so the factory can be demoed and tested without a
//! container runtime. Never point it at untrusted models or inputs; use the Docker sandbox.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{run_command, ExecResult, Sandbox, Workspace};
use crate::error::Result;

pub struct LocalSandbox {
    workspace: Workspace,
    home: PathBuf,
}

impl LocalSandbox {
    pub fn new(workspace: Workspace, home: PathBuf) -> Self {
        Self { workspace, home }
    }
}

#[async_trait]
impl Sandbox for LocalSandbox {
    fn kind(&self) -> &'static str {
        "local"
    }

    fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    async fn start(&self) -> Result<()> {
        std::fs::create_dir_all(self.workspace.root())?;
        std::fs::create_dir_all(&self.home)?;
        Ok(())
    }

    async fn exec(&self, command: &str, timeout: Duration) -> Result<ExecResult> {
        let root = self.workspace.root().canonicalize()?;
        let mut cmd = Command::new("timeout");
        cmd.args([
            "-k",
            "2",
            &timeout.as_secs().max(1).to_string(),
            "bash",
            "-c",
            command,
        ])
        .current_dir(&root)
        // Start from an empty environment: host secrets (API keys…) must not leak into
        // agent-run commands.
        .env_clear()
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        )
        .env("HOME", &self.home)
        .env("LANG", "C.UTF-8")
        .env("PYTHONPATH", &root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("CI", "1");
        run_command(cmd, command, timeout).await
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(dir: &tempfile::TempDir) -> LocalSandbox {
        LocalSandbox::new(
            Workspace::new(dir.path().join("workspace")),
            dir.path().join("home"),
        )
    }

    #[tokio::test]
    async fn runs_commands_in_the_workspace_with_a_clean_env() {
        let dir = tempfile::tempdir().unwrap();
        let sb = sandbox(&dir);
        sb.start().await.unwrap();
        std::env::set_var("OMNIFORGE_TEST_SECRET", "leak-me");
        let res = sb
            .exec(
                "echo hello > out.txt && cat out.txt && echo \"[${OMNIFORGE_TEST_SECRET:-}]\"",
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        assert!(res.ok(), "{res:?}");
        assert_eq!(res.stdout, "hello\n[]\n");
        assert_eq!(sb.workspace().read("out.txt").unwrap(), "hello\n");
    }

    #[tokio::test]
    async fn reports_failures_and_timeouts() {
        let dir = tempfile::tempdir().unwrap();
        let sb = sandbox(&dir);
        sb.start().await.unwrap();
        let failed = sb
            .exec("echo boom >&2; exit 3", Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(failed.exit_code, 3);
        assert_eq!(failed.combined(), "boom");
        let slow = sb.exec("sleep 30", Duration::from_secs(1)).await.unwrap();
        assert!(slow.timed_out);
        assert!(!slow.ok());
    }
}

//! Docker sandbox: one hardened, network-less container per run.
//!
//! Hardening applied by default: `--network none`, all capabilities dropped,
//! `no-new-privileges`, memory / CPU / PID limits, unprivileged user. The workspace is the
//! only writable mount. For stronger isolation (untrusted code at scale), swap the runtime
//! for gVisor (`--runtime runsc`), Kata/Firecracker microVMs or a remote runtime: only this
//! file changes, the [`Sandbox`] contract stays the same.

use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{run_command, ExecResult, Sandbox, Workspace};
use crate::config::SandboxConfig;
use crate::error::{Error, Result};

pub struct DockerSandbox {
    cfg: SandboxConfig,
    name: String,
    workspace: Workspace,
}

impl DockerSandbox {
    pub fn new(cfg: SandboxConfig, run_id: &str, workspace: Workspace) -> Self {
        let safe: String = run_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        Self {
            cfg,
            name: format!("omniforge-{safe}"),
            workspace,
        }
    }

    pub fn container_name(&self) -> &str {
        &self.name
    }

    async fn host_user() -> String {
        let id = |flag: &'static str| async move {
            let out = Command::new("id").arg(flag).output().await.ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
        };
        match (id("-u").await, id("-g").await) {
            (Some(u), Some(g)) => format!("{u}:{g}"),
            _ => "1000:1000".to_string(),
        }
    }

    /// `docker run` arguments. Kept as a pure function so the hardening is unit-tested
    /// without a Docker daemon.
    pub fn run_args(&self, user: &str) -> Result<Vec<String>> {
        let root = self.workspace.root().canonicalize()?;
        let mount = match &self.cfg.volume {
            // Docker-out-of-Docker: the API container shares a named volume with its
            // sandboxes; each sandbox only sees its own run sub-directory.
            Some(volume) => {
                let data_root = self.cfg.volume_root.as_deref().ok_or_else(|| {
                    Error::Invalid("sandbox.volume requires sandbox.volume_root".into())
                })?;
                let sub = root.strip_prefix(data_root).map_err(|_| {
                    Error::Invalid(format!(
                        "workspace {} is not under volume_root {data_root}",
                        root.display()
                    ))
                })?;
                // `volume-nocopy`: without it Docker "copies up" the image's /workspace
                // (owned by root) onto the empty sub-directory and the unprivileged
                // sandbox user can no longer write to its own workspace.
                format!(
                    "type=volume,src={volume},dst=/workspace,volume-subpath={},volume-nocopy",
                    sub.display()
                )
            }
            None => format!("type=bind,src={},dst=/workspace", root.display()),
        };
        let mut args: Vec<String> = [
            "run",
            "-d",
            "--rm",
            "--name",
            &self.name,
            "--label",
            "omniforge.sandbox=1",
            "--network",
            &self.cfg.network,
            "--memory",
            &self.cfg.memory,
            "--cpus",
            &self.cfg.cpus,
            "--pids-limit",
            &self.cfg.pids_limit.to_string(),
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--user",
            user,
            "--mount",
            &mount,
            "--workdir",
            "/workspace",
            "-e",
            "HOME=/tmp",
            "-e",
            "PYTHONDONTWRITEBYTECODE=1",
            "-e",
            "CI=1",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if let Some(runtime) = &self.cfg.runtime {
            args.extend(["--runtime".to_string(), runtime.clone()]);
        }
        args.extend([self.cfg.image.clone(), "sleep".into(), "infinity".into()]);
        Ok(args)
    }

    async fn docker(&self, args: &[String], timeout: Duration) -> Result<ExecResult> {
        let mut cmd = Command::new("docker");
        cmd.args(args);
        run_command(cmd, &format!("docker {}", args.join(" ")), timeout).await
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    fn kind(&self) -> &'static str {
        "docker"
    }

    fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    async fn start(&self) -> Result<()> {
        std::fs::create_dir_all(self.workspace.root())?;
        let user = match &self.cfg.user {
            Some(u) => u.clone(),
            None => Self::host_user().await,
        };
        // A previous crashed run may have left a container with the same name behind.
        let _ = self
            .docker(
                &["rm".into(), "-f".into(), self.name.clone()],
                Duration::from_secs(30),
            )
            .await;
        let res = self
            .docker(&self.run_args(&user)?, Duration::from_secs(120))
            .await?;
        if !res.ok() {
            return Err(Error::Sandbox(format!(
                "cannot start container from image '{}': {}",
                self.cfg.image,
                res.combined()
            )));
        }
        Ok(())
    }

    async fn exec(&self, command: &str, timeout: Duration) -> Result<ExecResult> {
        let mut cmd = Command::new("docker");
        cmd.args([
            "exec",
            "--workdir",
            "/workspace",
            &self.name,
            "timeout",
            "-k",
            "2",
        ])
        .arg(timeout.as_secs().max(1).to_string())
        .args(["bash", "-c", command]);
        run_command(cmd, command, timeout).await
    }

    async fn stop(&self) -> Result<()> {
        self.docker(
            &["rm".into(), "-f".into(), self.name.clone()],
            Duration::from_secs(30),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_is_hardened_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let sb = DockerSandbox::new(
            SandboxConfig::default(),
            "run-42",
            Workspace::new(dir.path()),
        );
        let args = sb.run_args("1000:1000").unwrap().join(" ");
        for expected in [
            "--network none",
            "--cap-drop ALL",
            "--security-opt no-new-privileges",
            "--pids-limit",
            "--memory",
            "--user 1000:1000",
            "--name omniforge-run-42",
            "dst=/workspace",
        ] {
            assert!(args.contains(expected), "missing `{expected}` in: {args}");
        }
        assert!(!args.contains("--privileged"));
        assert!(!args.contains("docker.sock"));
    }

    #[test]
    fn named_volume_mount_is_scoped_to_the_run_directory() {
        let dir = tempfile::tempdir().unwrap();
        let ws_dir = dir.path().join("runs/r1/workspace");
        std::fs::create_dir_all(&ws_dir).unwrap();
        let cfg = SandboxConfig {
            volume: Some("omniforge-data".into()),
            volume_root: Some(
                dir.path()
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..SandboxConfig::default()
        };
        let sb = DockerSandbox::new(cfg, "r1", Workspace::new(ws_dir));
        let args = sb.run_args("0:0").unwrap().join(" ");
        assert!(
            args.contains(
                "type=volume,src=omniforge-data,dst=/workspace,volume-subpath=runs/r1/workspace,volume-nocopy"
            ),
            "{args}"
        );
    }
}

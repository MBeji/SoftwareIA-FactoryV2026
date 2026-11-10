//! Docker sandbox tests against a real daemon.
//!
//! Opt-in: they need Docker and the sandbox image (`docker build -t omniforge-sandbox:latest
//! sandbox/`). Run them with `OMNIFORGE_TEST_DOCKER=1 cargo test --test docker`.

mod common;

use std::time::Duration;

use common::*;
use omniforge::config::{SandboxConfig, Settings};
use omniforge::governance::SupervisionMode;
use omniforge::orchestrator::state::RunStatus;
use omniforge::orchestrator::Orchestrator;
use omniforge::sandbox::docker::DockerSandbox;
use omniforge::sandbox::{Sandbox, Workspace};

fn enabled() -> bool {
    let on = std::env::var("OMNIFORGE_TEST_DOCKER").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set OMNIFORGE_TEST_DOCKER=1 to run Docker sandbox tests");
    }
    on
}

fn containers() -> String {
    let out = std::process::Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            "label=omniforge.sandbox=1",
            "--format",
            "{{.Names}}",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test]
async fn the_container_is_actually_isolated() {
    if !enabled() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::new(dir.path().join("workspace"));
    let sandbox = DockerSandbox::new(SandboxConfig::default(), "isolation-test", workspace);
    sandbox.start().await.unwrap();
    let run =
        |cmd: &'static str| async { sandbox.exec(cmd, Duration::from_secs(30)).await.unwrap() };

    // The workspace is shared with the host; nothing else is.
    assert!(run("echo data > from_container.txt && pwd")
        .await
        .stdout
        .contains("/workspace"));
    assert_eq!(
        sandbox.workspace().read("from_container.txt").unwrap(),
        "data\n"
    );
    sandbox
        .workspace()
        .write("from_host.txt", "hello\n")
        .unwrap();
    assert_eq!(run("cat from_host.txt").await.stdout, "hello\n");

    // No network at all: only the loopback interface exists.
    let interfaces = run("ls /sys/class/net").await.stdout;
    assert_eq!(
        interfaces.split_whitespace().collect::<Vec<_>>(),
        vec!["lo"],
        "unexpected interfaces: {interfaces}"
    );
    let egress =
        run("python3 -c \"import socket; socket.create_connection(('1.1.1.1', 443), timeout=3)\"")
            .await;
    assert!(!egress.ok(), "egress should be impossible: {egress:?}");

    // No capability, no privilege escalation, bounded process count.
    let status = run("cat /proc/self/status").await.stdout;
    assert!(status.contains("CapEff:\t0000000000000000"), "{status}");
    assert!(status.contains("NoNewPrivs:\t1"), "{status}");
    assert_eq!(
        run("cat /sys/fs/cgroup/pids.max 2>/dev/null || cat /sys/fs/cgroup/pids/pids.max")
            .await
            .stdout
            .trim(),
        "256"
    );

    // The toolchain agents rely on is present, and runaway commands are killed.
    assert!(
        run("git --version && pytest --version && ruff --version && ast-grep --version")
            .await
            .ok()
    );
    let slow = sandbox
        .exec("sleep 60", Duration::from_secs(1))
        .await
        .unwrap();
    assert!(slow.timed_out);

    sandbox.stop().await.unwrap();
    assert!(!containers().contains(sandbox.container_name()));
}

#[tokio::test]
async fn a_full_run_executes_in_the_container_and_cleans_up() {
    if !enabled() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings::for_tests(dir.path());
    settings.sandbox = Some("docker".into());
    let orchestrator = Orchestrator::new(settings).unwrap();
    let run = orchestrator
        .start_run(request(JWT_TASK, policy(SupervisionMode::Autonomous)))
        .unwrap();
    let (state, _) = drive(&orchestrator, &run.id, |_| panic!("autonomous")).await;

    assert_eq!(state.status, RunStatus::Completed, "{:?}", state.error);
    assert_eq!(state.sandbox, "docker");
    assert!(orchestrator
        .workspace(&run.id)
        .unwrap()
        .read("app/jwt_auth.py")
        .unwrap()
        .contains("compare_digest"));
    assert!(
        !containers().contains(&run.id),
        "the sandbox container must be removed when the run ends"
    );
}

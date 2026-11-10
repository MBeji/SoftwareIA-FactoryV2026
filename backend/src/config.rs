//! Configuration: runtime settings (environment / CLI) and the factory profile
//! (`config/omniforge.toml`: routing table, default supervision policy, sandbox limits).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::governance::SupervisionPolicy;
use crate::llm::router::RoutingConfig;

/// Which LLM backend serves the agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum LlmMode {
    /// Deterministic offline double: scripted agents, real sandbox. No API key needed.
    Simulated,
    /// Any OpenAI-compatible gateway — in practice the LiteLLM proxy shipped in this repo.
    Gateway,
}

/// Process-level settings. Every flag can also be set through its environment variable.
#[derive(Debug, Clone, clap::Args)]
pub struct Settings {
    /// Directory holding the event store and the run workspaces.
    #[arg(long, env = "OMNIFORGE_DATA_DIR", default_value = "./data")]
    pub data_dir: PathBuf,

    /// Factory profile (routing, supervision, sandbox). Built-in defaults when absent.
    #[arg(long, env = "OMNIFORGE_CONFIG")]
    pub config: Option<PathBuf>,

    /// `simulated` (offline demo) or `gateway` (LiteLLM proxy / OpenAI-compatible endpoint).
    #[arg(
        long,
        env = "OMNIFORGE_LLM_MODE",
        value_enum,
        default_value = "simulated"
    )]
    pub llm_mode: LlmMode,

    /// Base URL of the OpenAI-compatible gateway, e.g. `http://litellm:4000/v1`.
    #[arg(
        long,
        env = "OMNIFORGE_LLM_BASE_URL",
        default_value = "http://localhost:4000/v1"
    )]
    pub llm_base_url: String,

    /// Gateway key (LiteLLM virtual key). Provider keys never reach this process.
    #[arg(long, env = "OMNIFORGE_LLM_API_KEY", hide_env_values = true)]
    pub llm_api_key: Option<String>,

    /// Sandbox backend override: `docker` or `local` (no isolation — demos and tests only).
    #[arg(long, env = "OMNIFORGE_SANDBOX")]
    pub sandbox: Option<String>,

    /// Artificial latency per simulated LLM call, so the dashboard is watchable.
    #[arg(long, env = "OMNIFORGE_SIM_DELAY_MS", default_value_t = 0)]
    pub sim_delay_ms: u64,
}

impl Settings {
    /// Settings for tests and embedded use: simulated LLM, local sandbox.
    pub fn for_tests(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            config: None,
            llm_mode: LlmMode::Simulated,
            llm_base_url: "http://localhost:4000/v1".into(),
            llm_api_key: None,
            sandbox: Some("local".into()),
            sim_delay_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// Default backend: `docker` (isolated) or `local` (no isolation).
    pub kind: String,
    pub image: String,
    /// Docker network for sandboxes. `none` = no egress at all.
    pub network: String,
    pub memory: String,
    pub cpus: String,
    pub pids_limit: u32,
    /// `uid:gid` inside the container. Defaults to the host user so files stay readable.
    pub user: Option<String>,
    /// Alternative OCI runtime, e.g. `runsc` (gVisor) or `kata-runtime`.
    pub runtime: Option<String>,
    /// Named volume shared with the API container (Docker-out-of-Docker deployments).
    pub volume: Option<String>,
    /// Mount point of that volume in the API container (= the data directory).
    pub volume_root: Option<String>,
    pub exec_timeout_s: u64,
    pub test_command: String,
    pub lint_command: String,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            kind: "docker".into(),
            image: "omniforge-sandbox:latest".into(),
            network: "none".into(),
            memory: "1g".into(),
            cpus: "1.0".into(),
            pids_limit: 256,
            user: None,
            runtime: None,
            volume: None,
            volume_root: None,
            exec_timeout_s: 120,
            test_command: "pytest -q --tb=short -p no:cacheprovider".into(),
            lint_command:
                "if command -v ruff >/dev/null 2>&1; then ruff check --no-cache .; else python3 -m compileall -q .; fi"
                    .into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanningConfig {
    /// Story points a sprint can absorb when building the autonomous sprint plan.
    pub sprint_capacity_points: u32,
}

impl Default for PlanningConfig {
    fn default() -> Self {
        Self {
            sprint_capacity_points: 13,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    /// Observe-Think-Act iterations an agent may spend on one assignment.
    pub max_steps: u32,
    /// Failed GREEN attempts before the developer is escalated to a stronger model tier.
    pub escalate_after_failures: u32,
    /// Automatic fix rounds for blocking security findings before raising an exception.
    pub max_security_fix_rounds: u32,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            max_steps: 16,
            escalate_after_failures: 2,
            max_security_fix_rounds: 2,
        }
    }
}

/// The factory profile. `Default` is the reference configuration; `config/omniforge.toml`
/// documents the same values and is the place to override them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FactoryConfig {
    pub routing: RoutingConfig,
    pub supervision: SupervisionPolicy,
    pub sandbox: SandboxConfig,
    pub planning: PlanningConfig,
    pub agents: AgentsConfig,
}

impl FactoryConfig {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let cfg = match path {
            Some(p) => {
                let raw = std::fs::read_to_string(p).map_err(|e| {
                    Error::Invalid(format!("cannot read config {}: {e}", p.display()))
                })?;
                toml::from_str(&raw)
                    .map_err(|e| Error::Invalid(format!("invalid config {}: {e}", p.display())))?
            }
            None => Self::default(),
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        self.supervision.validate()?;
        self.routing.validate()?;
        if self.planning.sprint_capacity_points == 0 {
            return Err(Error::Invalid(
                "planning.sprint_capacity_points must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped profile must stay in sync with the built-in defaults.
    #[test]
    fn shipped_profile_matches_builtin_defaults() {
        let shipped: FactoryConfig =
            toml::from_str(include_str!("../../config/omniforge.toml")).unwrap();
        shipped.validate().unwrap();
        assert_eq!(shipped, FactoryConfig::default());
    }
}

//! # OmniForge AI — agentic software factory (core)
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`events`] | Append-only event stream + store (the system's source of truth) |
//! | [`llm`] | Provider-agnostic LLM layer: router, LiteLLM gateway client, offline double |
//! | [`sandbox`] | Isolated execution (Docker) and traversal-safe workspace access |
//! | [`tools`] | Agent tools, permission-checked and audited |
//! | [`agents`] | Role specifications and the Observe-Think-Act runtime |
//! | [`ingestion`] | Input classification (3 levels), HTN planning, risks, sprints |
//! | [`governance`] | Supervision matrix (HITL / HOTL / autonomous), gates, security scanner |
//! | [`orchestrator`] | SDLC state machine, TDD loop, run lifecycle |
//! | [`memory`] | Vector memory of lessons and decisions |
//! | [`api`] | HTTP + WebSocket API |

pub mod agents;
pub mod api;
pub mod config;
pub mod error;
pub mod events;
pub mod governance;
pub mod ingestion;
pub mod llm;
pub mod memory;
pub mod orchestrator;
pub mod sandbox;
pub mod tools;

pub use error::{Error, Result};

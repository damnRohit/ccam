//! ccam: keep several Claude subscription logins and switch Claude Code between them.
//!
//! The design and its invariants are in `docs/SPEC.md`.

pub mod api;
pub mod auto;
pub mod autorun;
pub mod config;
pub mod error;
pub mod hooks;
pub mod jsonio;
pub mod launch;
pub mod lock;
pub mod login;
pub mod manager;
pub mod name;
pub mod notify;
pub mod paths;
pub mod presence;
#[cfg(windows)]
pub mod restart;
pub mod store;
#[cfg(test)]
mod testserver;
pub mod usage;
pub mod vault;
pub mod warmup;

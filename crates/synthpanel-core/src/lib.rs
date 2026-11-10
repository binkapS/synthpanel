//! SynthPanel privileged core daemon.
//!
//! The core is the only component that runs as root and the only component
//! allowed to mutate system state. It exposes a Unix-socket IPC API consumed
//! by `synthpanel-web` and `synthpanel-cli`.

pub mod audit;
pub mod auth;
pub mod config;
pub mod db;
pub mod ipc;
pub mod metrics;
pub mod settings;
pub mod users;

pub use config::Paths;

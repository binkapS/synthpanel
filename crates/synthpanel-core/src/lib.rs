//! SynthPanel privileged core daemon.
//!
//! The core is the only component that runs as root and the only component
//! allowed to mutate system state. It exposes a Unix-socket IPC API consumed
//! by `synthpanel-web` and `synthpanel-cli`.

pub mod alerts;
pub mod apply;
pub mod audit;
pub mod auth;
pub mod backup;
pub mod config;
pub mod cron;
pub mod database;
pub mod databases;
pub mod db;
pub mod deploy;
pub mod files;
pub mod ipc;
pub mod logs;
pub mod mailer;
pub mod metrics;
pub mod nginx;
pub mod php;
pub mod scheduler;
pub mod security;
pub mod settings;
pub mod sites;
pub mod ssl;
pub mod terminal;
pub mod update;
pub mod users;

pub use config::Paths;

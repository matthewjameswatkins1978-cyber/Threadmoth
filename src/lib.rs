#![forbid(unsafe_code)]

pub mod benchmark;
pub mod build_info;
pub mod capabilities;
pub mod diff_planner;
pub mod engine;
pub mod lifecycle;
pub mod metadata;
pub mod path;
pub mod pattern;
pub mod pipeline;
mod presentation;
pub mod protocol;
pub mod provider;
pub mod recovery;
pub mod shorthand;
pub mod source_prepare;
pub mod target_registry;
pub mod torture;
pub mod updater;
pub mod workspace;

pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

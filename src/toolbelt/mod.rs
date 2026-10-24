//! Threadmoth Toolbelt: extensible attachments for AI coding agents.
//!
//! Guiding principle: Different instruments, one dependable editing contract.
//! Attachments advertise their capabilities and propose changes against observed source snapshots;
//! the Threadmoth host retains exclusive mutation authority, ensuring workspace containment,
//! pre-hash verification, atomic replacement, effect budgets, and rollback safety.

pub mod structural;

use crate::workspace::Workspace;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Metadata advertising an attachment's capabilities, engine, prerequisites and boundaries.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct AttachmentInfo {
    pub name: String,
    pub engine: String,
    pub version: Option<String>,
    pub available: bool,
    pub description: String,
    pub supported_languages: Vec<String>,
    pub prerequisites: String,
    pub limitations: Vec<String>,
}

/// Discover all available attachments in the current workspace environment.
pub fn discover_toolbelt(workspace: &Workspace) -> Vec<AttachmentInfo> {
    vec![structural::discover(workspace)]
}

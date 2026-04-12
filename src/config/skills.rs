use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Skills system configuration.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    pub enabled: bool,
    /// Directory containing user-placed skills (default: ~/.ironclaw/skills/).
    /// Skills here are loaded with `Trusted` trust level.
    pub local_dir: PathBuf,
    /// Directory containing registry-installed skills (default: ~/.ironclaw/installed_skills/).
    /// Skills here are loaded with `Installed` trust level and get read-only tool access.
    pub installed_dir: PathBuf,
    /// Optional project-local workspace skills directory (default: ./skills/ relative to cwd).
    /// Set via `SKILLS_WORKSPACE_DIR`. Skills here are loaded with `Trusted` trust level.
    pub workspace_skills_dir: Option<PathBuf>,
    /// Maximum number of skills that can be active simultaneously.
    pub max_active_skills: usize,
    /// Maximum total context tokens allocated to skill prompts.
    pub max_context_tokens: usize,
    /// Maximum recursion depth when scanning skill directories for bundle layouts.
    /// Subdirectories without `SKILL.md` are recursed into up to this depth.
    pub max_scan_depth: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            local_dir: default_skills_dir(),
            installed_dir: default_installed_skills_dir(),
            workspace_skills_dir: default_workspace_skills_dir(),
            max_active_skills: 5,
            max_context_tokens: 8000,
            max_scan_depth: 3,
        }
    }
}

/// Get the default user skills directory (~/.ironclaw/skills/).
fn default_skills_dir() -> PathBuf {
    ironclaw_base_dir().join("skills")
}

/// Get the default installed skills directory (~/.ironclaw/installed_skills/).
fn default_installed_skills_dir() -> PathBuf {
    ironclaw_base_dir().join("installed_skills")
}

/// Returns `./skills` relative to the current working directory if it exists,
/// otherwise `None`. This enables project-local skill bundles (e.g. Personifi skills)
/// to be discovered automatically when running from the project root.
fn default_workspace_skills_dir() -> Option<PathBuf> {
    let dir = std::env::current_dir().ok()?.join("skills");
    dir.is_dir().then_some(dir)
}

impl SkillsConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("SKILLS_ENABLED", true)?,
            local_dir: optional_env("SKILLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_skills_dir),
            installed_dir: optional_env("SKILLS_INSTALLED_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_installed_skills_dir),
            workspace_skills_dir: optional_env("SKILLS_WORKSPACE_DIR")?
                .map(PathBuf::from)
                .or_else(default_workspace_skills_dir),
            max_active_skills: parse_optional_env("SKILLS_MAX_ACTIVE", 5)?,
            max_context_tokens: parse_optional_env("SKILLS_MAX_CONTEXT_TOKENS", 8000)?,
            max_scan_depth: parse_optional_env("SKILLS_MAX_SCAN_DEPTH", 3)?,
        })
    }
}

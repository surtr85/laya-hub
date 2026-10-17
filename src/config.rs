use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_version")]
    pub version: String,
    pub laya: LayaConfig,
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(default)]
    pub skills: HashMap<String, SkillConfig>,
}

fn default_version() -> String {
    "1.0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayaConfig {
    #[serde(default = "default_laya_url")]
    pub url: String,
    #[serde(default = "default_threshold")]
    pub threshold: f32,
    #[serde(default = "default_max_active_tools")]
    pub max_active_tools: usize,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

fn default_laya_url() -> String {
    "http://127.0.0.1:8089/v1/systemone".to_string()
}

fn default_threshold() -> f32 {
    0.60
}

fn default_max_active_tools() -> usize {
    5
}

fn default_idle_timeout_secs() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpServerConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        cwd: Option<String>,
        description: String,
        #[serde(default)]
        tags: Vec<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        description: String,
        #[serde(default)]
        tags: Vec<String>,
    },
}

impl McpServerConfig {
    pub fn description(&self) -> &str {
        match self {
            McpServerConfig::Stdio { description, .. } => description,
            McpServerConfig::Http { description, .. } => description,
        }
    }

    pub fn tags(&self) -> &[String] {
        match self {
            McpServerConfig::Stdio { tags, .. } => tags,
            McpServerConfig::Http { tags, .. } => tags,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    pub path: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl AppConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file at {:?}", path.as_ref()))?;
        Self::parse_str(&content)
    }

    pub fn parse_str(raw: &str) -> Result<Self> {
        let mut config: AppConfig = serde_yaml::from_str(raw)
            .context("Failed to deserialize YAML configuration")?;
        
        config.resolve_env_and_paths()?;
        Ok(config)
    }

    fn resolve_env_and_paths(&mut self) -> Result<()> {
        // Expand skills paths
        for skill in self.skills.values_mut() {
            let expanded = shellexpand::tilde(&skill.path).to_string();
            skill.path = expanded;
        }

        // Expand environment variables in stdio and http servers
        for server in self.mcp_servers.values_mut() {
            match server {
                McpServerConfig::Stdio { command, cwd, env, .. } => {
                    *command = resolve_val(command);
                    if let Some(c) = cwd {
                        *c = shellexpand::tilde(c).to_string();
                    }
                    for val in env.values_mut() {
                        *val = resolve_val(val);
                    }
                }
                McpServerConfig::Http { url, headers, .. } => {
                    *url = resolve_val(url);
                    for val in headers.values_mut() {
                        *val = resolve_val(val);
                    }
                }
            }
        }

        Ok(())
    }
}

fn resolve_val(input: &str) -> String {
    let trimmed = input.trim();

    // Support "Bearer <prefix>..." recursive expansion
    if let Some(rest) = trimmed.strip_prefix("Bearer ") {
        let inner = resolve_val(rest);
        if inner.is_empty() {
            return String::new();
        }
        return format!("Bearer {}", inner);
    }

    if let Some(rest) = trimmed.strip_prefix("file:") {
        let path = shellexpand::tilde(rest).to_string();
        if let Ok(content) = std::fs::read_to_string(&path) {
            let val = content.trim().to_string();
            if !val.is_empty() {
                return val;
            }
        }
        return String::new();
    }

    if let Some(rest) = trimmed.strip_prefix("env:") {
        if let Ok(val) = std::env::var(rest) {
            let val = val.trim().to_string();
            if !val.is_empty() {
                return val;
            }
        }
        // Fallback for COMPOSIO_API_KEY if env var is unset
        if rest == "COMPOSIO_API_KEY" {
            let fallback_path = shellexpand::tilde("~/.config/composio/api_key").to_string();
            if let Ok(content) = std::fs::read_to_string(&fallback_path) {
                let val = content.trim().to_string();
                if !val.is_empty() {
                    return val;
                }
            }
        }
        return String::new();
    }

    // Support standard $VAR or ${VAR} expansion
    shellexpand::env(input)
        .unwrap_or_else(|_| input.into())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_val_env_missing() {
        let val = resolve_val("env:DEFINITELY_NON_EXISTENT_VAR_12345");
        assert_eq!(val, "");
    }

    #[test]
    fn test_resolve_val_env_present() {
        std::env::set_var("TEST_FOO_BAR_LAYA", "hello_world");
        let val = resolve_val("env:TEST_FOO_BAR_LAYA");
        assert_eq!(val, "hello_world");
    }
}

pub fn read_skill_content(path_str: &str) -> Result<String> {
    let path = PathBuf::from(shellexpand::tilde(path_str).as_ref());
    std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read skill Markdown file at {:?}", path))
}

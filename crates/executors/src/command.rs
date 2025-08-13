use std::{collections::HashMap, fs, path::PathBuf, sync::OnceLock};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::executors::CodingAgent;

static PROFILES_CACHE: OnceLock<AgentProfiles> = OnceLock::new();

// Default profiles embedded at compile time
const DEFAULT_PROFILES_JSON: &str = include_str!("../default_profiles.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct CommandBuilder {
    /// Base executable command (e.g., "npx -y @anthropic-ai/claude-code@latest")
    pub base: String,
    /// Optional parameters to append to the base command
    pub params: Option<Vec<String>>,
}

impl CommandBuilder {
    pub fn new<S: Into<String>>(base: S) -> Self {
        Self {
            base: base.into(),
            params: None,
        }
    }

    pub fn params<I>(mut self, params: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        self.params = Some(params.into_iter().map(|p| p.into()).collect());
        self
    }
    pub fn build_initial(&self) -> String {
        let mut parts = vec![self.base.clone()];
        if let Some(ref params) = self.params {
            parts.extend(params.clone());
        }
        parts.join(" ")
    }

    pub fn build_follow_up(&self, additional_args: &[String]) -> String {
        let mut parts = vec![self.base.clone()];
        if let Some(ref params) = self.params {
            parts.extend(params.clone());
        }
        parts.extend(additional_args.iter().cloned());
        parts.join(" ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct AgentVariantProfile {
    /// Unique identifier for this profile (e.g., "MyClaudeCode", "FastAmp")
    pub label: String,
    /// The coding agent this profile is associated with
    #[serde(flatten)]
    pub agent: CodingAgent,
    /// Optional profile-specific MCP config file path (absolute; supports leading ~). Overrides the default `BaseCodingAgent` config path
    pub mcp_config_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct AgentProfile {
    /// Unique identifier for this profile (e.g., "MyClaudeCode", "FastAmp")
    pub label: String,
    /// The coding agent this profile is associated with
    #[serde(flatten)]
    pub agent: CodingAgent,
    /// Optional profile-specific MCP config file path (absolute; supports leading ~). Overrides the default `BaseCodingAgent` config path
    pub mcp_config_path: Option<String>,
    /// Supported modes for this profile, may be empty
    pub variants: Vec<AgentVariantProfile>,
}

impl AgentProfile {
    pub fn get_variant(&self, variant: &str) -> Option<&AgentVariantProfile> {
        self.variants.iter().find(|m| m.label == variant)
    }

    pub fn get_mcp_config_path(&self) -> Option<PathBuf> {
        match self.mcp_config_path.as_ref() {
            Some(path) => Some(PathBuf::from(path)),
            None => self.agent.default_mcp_config_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct ProfileVariant {
    pub profile: String,
    pub variant: Option<String>,
}

impl ProfileVariant {
    pub fn default(profile: String) -> Self {
        Self {
            profile,
            variant: None,
        }
    }
    pub fn with_variant(profile: String, mode: String) -> Self {
        Self {
            profile,
            variant: Some(mode),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct AgentProfiles {
    pub profiles: Vec<AgentProfile>,
}

impl AgentProfiles {
    pub fn get_cached() -> &'static AgentProfiles {
        PROFILES_CACHE.get_or_init(Self::load)
    }

    fn load() -> Self {
        let profiles_path = utils::assets::profiles_path();

        // load from profiles.json if it exists, otherwise use defaults
        let content = match fs::read_to_string(&profiles_path) {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!("Failed to read profiles.json: {}, using defaults", e);
                return Self::from_defaults();
            }
        };

        match serde_json::from_str::<Self>(&content) {
            Ok(profiles) => {
                tracing::info!("Loaded all profiles from profiles.json");
                profiles
            }
            Err(e) => {
                tracing::warn!("Failed to parse profiles.json: {}, using defaults", e);
                Self::from_defaults()
            }
        }
    }

    pub fn from_defaults() -> Self {
        serde_json::from_str(DEFAULT_PROFILES_JSON).unwrap_or_else(|e| {
            tracing::error!("Failed to parse embedded default_profiles.json: {}", e);
            panic!("Default profiles JSON is invalid")
        })
    }

    pub fn get_profile(&self, label: &str) -> Option<&AgentProfile> {
        self.profiles.iter().find(|p| p.label == label)
    }

    pub fn to_map(&self) -> HashMap<String, AgentProfile> {
        self.profiles
            .iter()
            .map(|p| (p.label.clone(), p.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_profiles_have_expected_base_and_noninteractive_or_json_flags() {
        // Build default profiles and make lookup by label easy
        let profiles = AgentProfiles::from_defaults().to_map();

        let get_profile_command = |label: &str| {
            profiles
                .get(label)
                .map(|p| {
                    use crate::executors::CodingAgent;
                    match &p.agent {
                        CodingAgent::ClaudeCode(claude) => claude.command.build_initial(),
                        CodingAgent::Amp(amp) => amp.command.build_initial(),
                        CodingAgent::Gemini(gemini) => gemini.command.build_initial(),
                        CodingAgent::Codex(codex) => codex.command.build_initial(),
                        CodingAgent::Opencode(opencode) => opencode.command.build_initial(),
                    }
                })
                .unwrap_or_else(|| panic!("Profile not found: {label}"))
        };
        let profiles = AgentProfiles::from_defaults();
        assert!(profiles.profiles.len() == 7);

        let claude_code_command = get_profile_command("claude-code");
        assert!(claude_code_command.contains("npx -y @anthropic-ai/claude-code@latest"));
        assert!(claude_code_command.contains("-p"));
        assert!(claude_code_command.contains("--dangerously-skip-permissions"));

        let claude_code_router_command = get_profile_command("claude-code-router");
        assert!(claude_code_router_command.contains("npx -y @musistudio/claude-code-router code"));
        assert!(claude_code_router_command.contains("-p"));
        assert!(claude_code_router_command.contains("--dangerously-skip-permissions"));

        let amp_command = get_profile_command("amp");
        assert!(amp_command.contains("npx -y @sourcegraph/amp@0.0.1752148945-gd8844f"));
        assert!(amp_command.contains("--format=jsonl"));

        let gemini_command = get_profile_command("gemini");
        assert!(gemini_command.contains("npx -y @google/gemini-cli@latest"));
        assert!(gemini_command.contains("--yolo"));

        let codex_command = get_profile_command("codex");
        assert!(codex_command.contains("npx -y @openai/codex exec"));
        assert!(codex_command.contains("--json"));

        let qwen_code_command = get_profile_command("qwen-code");
        assert!(qwen_code_command.contains("npx -y @qwen-code/qwen-code@latest"));
        assert!(qwen_code_command.contains("--yolo"));

        let opencode_command = get_profile_command("opencode");
        assert!(opencode_command.contains("npx -y opencode-ai@latest run"));
        assert!(opencode_command.contains("--print-logs"));
    }

    #[test]
    fn test_flattened_agent_deserialization() {
        let test_json = r#"{
            "profiles": [
                {
                    "label": "test-claude",
                    "mcp_config_path": null,
                    "CLAUDE_CODE": {
                        "command": {
                            "base": "npx claude",
                            "params": ["--test"]
                        },
                        "plan": true
                    }
                },
                {
                    "label": "test-gemini",
                    "mcp_config_path": null,
                    "GEMINI": {
                        "command_builder": {
                            "base": "npx gemini",
                            "params": ["--test"]
                        }
                    }
                }
            ]
        }"#;

        let profiles: AgentProfiles = serde_json::from_str(test_json).expect("Should deserialize");
        assert_eq!(profiles.profiles.len(), 2);

        // Test Claude profile
        let claude_profile = profiles.get_profile("test-claude").unwrap();
        match &claude_profile.agent {
            crate::executors::CodingAgent::ClaudeCode(claude) => {
                assert_eq!(claude.command.base, "npx claude");
                assert_eq!(claude.command.params.as_ref().unwrap()[0], "--test");
                assert_eq!(claude.plan, true);
            }
            _ => panic!("Expected ClaudeCode agent"),
        }

        // Test Gemini profile
        let gemini_profile = profiles.get_profile("test-gemini").unwrap();
        match &gemini_profile.agent {
            crate::executors::CodingAgent::Gemini(gemini) => {
                assert_eq!(gemini.command.base, "npx gemini");
                assert_eq!(gemini.command.params.as_ref().unwrap()[0], "--test");
            }
            _ => panic!("Expected Gemini agent"),
        }
    }
}

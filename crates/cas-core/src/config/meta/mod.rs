//! Configuration metadata registry
//!
//! Provides comprehensive metadata for all CAS configuration options including:
//! - Descriptions and documentation
//! - Types and validation constraints
//! - Default values
//! - Section organization
//!
//! This enables rich CLI interfaces like `cas config describe`, validation,
//! shell completion, and interactive editors.

use std::collections::HashMap;

/// Type of configuration value
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigType {
    /// Boolean (true/false)
    Bool,
    /// Integer number
    Int,
    /// String value
    String,
    /// Comma-separated list of strings
    StringList,
}

impl ConfigType {
    /// Get a human-readable type name
    pub fn name(&self) -> &'static str {
        match self {
            ConfigType::Bool => "boolean",
            ConfigType::Int => "integer",
            ConfigType::String => "string",
            ConfigType::StringList => "string list",
        }
    }

    /// Get example values for this type
    pub fn examples(&self) -> Vec<&'static str> {
        match self {
            ConfigType::Bool => vec!["true", "false"],
            ConfigType::Int => vec!["10", "100", "1000"],
            ConfigType::String => vec!["value", "path/to/file"],
            ConfigType::StringList => vec!["item1,item2,item3"],
        }
    }
}

/// Validation constraints for a config value
#[derive(Debug, Clone)]
pub enum Constraint {
    /// No constraints
    None,
    /// Minimum integer value
    Min(i64),
    /// Maximum integer value
    Max(i64),
    /// Range (inclusive)
    Range(i64, i64),
    /// Must be one of these values
    OneOf(Vec<String>),
    /// Must not be empty
    NotEmpty,
    /// Must be a valid path
    ValidPath,
}

/// Metadata for a single configuration option
#[derive(Debug, Clone)]
pub struct ConfigMeta {
    /// The full key path (e.g., "hooks.token_budget")
    pub key: &'static str,
    /// Section this belongs to
    pub section: &'static str,
    /// Human-readable display name
    pub name: &'static str,
    /// Detailed description
    pub description: &'static str,
    /// Value type
    pub value_type: ConfigType,
    /// Default value as string
    pub default: &'static str,
    /// Validation constraint
    pub constraint: Constraint,
    /// Whether this is an advanced option (hidden by default in simple lists)
    pub advanced: bool,
    /// Feature flag required (if any)
    pub requires_feature: Option<&'static str>,
}

impl ConfigMeta {
    /// Validate a value against this config's constraints
    pub fn validate(&self, value: &str) -> Result<(), String> {
        // First check type
        match self.value_type {
            ConfigType::Bool => {
                if !["true", "false", "1", "0", "yes", "no"].contains(&value.to_lowercase().as_str()) {
                    return Err(format!("Expected boolean (true/false), got: {}", value));
                }
            }
            ConfigType::Int => {
                let parsed: Result<i64, _> = value.parse();
                if parsed.is_err() {
                    return Err(format!("Expected integer, got: {}", value));
                }
                let num = parsed.unwrap();

                // Check constraints
                match &self.constraint {
                    Constraint::Min(min) => {
                        if num < *min {
                            return Err(format!("Value must be at least {}, got: {}", min, num));
                        }
                    }
                    Constraint::Max(max) => {
                        if num > *max {
                            return Err(format!("Value must be at most {}, got: {}", max, num));
                        }
                    }
                    Constraint::Range(min, max) => {
                        if num < *min || num > *max {
                            return Err(format!("Value must be between {} and {}, got: {}", min, max, num));
                        }
                    }
                    _ => {}
                }
            }
            ConfigType::String | ConfigType::StringList => {
                match &self.constraint {
                    Constraint::NotEmpty => {
                        if value.trim().is_empty() {
                            return Err("Value cannot be empty".to_string());
                        }
                    }
                    Constraint::OneOf(options) => {
                        if !options.iter().any(|o| o.eq_ignore_ascii_case(value)) {
                            return Err(format!(
                                "Value must be one of: {}",
                                options.join(", ")
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    /// Format value for display
    pub fn format_value(&self, value: &str) -> String {
        match self.value_type {
            ConfigType::Bool => {
                if value == "true" || value == "1" || value.to_lowercase() == "yes" {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            ConfigType::StringList => {
                // Format as comma-separated, handling empty
                if value.is_empty() {
                    "(empty)".to_string()
                } else {
                    value.to_string()
                }
            }
            _ => value.to_string(),
        }
    }

    /// Check if value differs from default
    pub fn is_modified(&self, value: &str) -> bool {
        value != self.default
    }
}

/// Registry of all configuration options
pub struct ConfigRegistry {
    /// All config metadata indexed by key
    configs: HashMap<&'static str, ConfigMeta>,
    /// Config keys organized by section
    sections: HashMap<&'static str, Vec<&'static str>>,
    /// Section descriptions
    section_descriptions: HashMap<&'static str, &'static str>,
}


mod seed;

impl ConfigRegistry {
    /// Create the complete config registry with all options
    pub fn new() -> Self {
        let mut registry = Self {
            configs: HashMap::new(),
            sections: HashMap::new(),
            section_descriptions: HashMap::new(),
        };
        registry.register_defaults();
        registry
    }

    /// Register a config option
    fn register(&mut self, meta: ConfigMeta) {
        self.sections
            .entry(meta.section)
            .or_default()
            .push(meta.key);
        self.configs.insert(meta.key, meta);
    }

    /// Get metadata for a config key
    pub fn get(&self, key: &str) -> Option<&ConfigMeta> {
        self.configs.get(key)
    }

    /// Get all config keys
    pub fn all_keys(&self) -> Vec<&'static str> {
        self.configs.keys().copied().collect()
    }

    /// Get keys for a section
    pub fn section_keys(&self, section: &str) -> Vec<&'static str> {
        self.sections.get(section).cloned().unwrap_or_default()
    }

    /// Get all section names
    pub fn sections(&self) -> Vec<&'static str> {
        let mut sections: Vec<_> = self.sections.keys().copied().collect();
        // Sort with parent sections before subsections
        sections.sort_by(|a, b| {
            let a_depth = a.matches('.').count();
            let b_depth = b.matches('.').count();
            if a_depth != b_depth {
                a_depth.cmp(&b_depth)
            } else {
                a.cmp(b)
            }
        });
        sections
    }

    /// Get section description
    pub fn section_description(&self, section: &str) -> Option<&'static str> {
        self.section_descriptions.get(section).copied()
    }

    /// Get all configs in a section
    pub fn configs_in_section(&self, section: &str) -> Vec<&ConfigMeta> {
        self.section_keys(section)
            .iter()
            .filter_map(|key| self.configs.get(key))
            .collect()
    }

    /// Search configs by keyword
    pub fn search(&self, query: &str) -> Vec<&ConfigMeta> {
        let query_lower = query.to_lowercase();
        self.configs
            .values()
            .filter(|meta| {
                meta.key.to_lowercase().contains(&query_lower)
                    || meta.name.to_lowercase().contains(&query_lower)
                    || meta.description.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    /// Get count of all config options
    pub fn count(&self) -> usize {
        self.configs.len()
    }

    /// Get only non-advanced configs
    pub fn basic_configs(&self) -> Vec<&ConfigMeta> {
        self.configs.values().filter(|m| !m.advanced).collect()
    }

    /// Get only advanced configs
    pub fn advanced_configs(&self) -> Vec<&ConfigMeta> {
        self.configs.values().filter(|m| m.advanced).collect()
    }

    /// Validate a value for a key
    pub fn validate(&self, key: &str, value: &str) -> Result<(), String> {
        match self.get(key) {
            Some(meta) => meta.validate(value),
            None => Err(format!("Unknown config key: {}", key)),
        }
    }
}

impl Default for ConfigRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global config registry singleton
static REGISTRY: std::sync::OnceLock<ConfigRegistry> = std::sync::OnceLock::new();

/// Get the global config registry
pub fn registry() -> &'static ConfigRegistry {
    REGISTRY.get_or_init(ConfigRegistry::new)
}

#[cfg(test)]
mod tests;

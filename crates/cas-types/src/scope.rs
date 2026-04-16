//! Scope type definitions for two-tier (global + project) architecture
//!
//! Scope determines where data is stored:
//! - Global: User-level data in ~/.config/cas/ (preferences, general learnings)
//! - Project: Project-level data in ./.cas/ (technical context, tasks)

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::TypeError;

/// Storage scope for CAS data
///
/// Determines whether data is stored globally (user-level) or per-project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Global scope - stored in ~/.config/cas/
    /// Used for: user preferences, general learnings, global skills
    Global,

    /// Project scope - stored in ./.cas/
    /// Used for: technical context, tasks, project-specific rules
    #[default]
    Project,
}

impl Scope {
    /// Get the ID prefix for this scope
    pub fn id_prefix(&self) -> &'static str {
        match self {
            Scope::Global => "g",
            Scope::Project => "p",
        }
    }

    /// Parse scope from ID prefix
    pub fn from_id_prefix(prefix: &str) -> Option<Self> {
        match prefix {
            "g" => Some(Scope::Global),
            "p" => Some(Scope::Project),
            _ => None,
        }
    }

    /// Extract scope from a prefixed ID (e.g., "g-2025-01-01-001" -> Global)
    pub fn from_id(id: &str) -> Option<Self> {
        if let Some(prefix) = id.split('-').next() {
            Self::from_id_prefix(prefix)
        } else {
            None
        }
    }

    /// Check if this is global scope
    pub fn is_global(&self) -> bool {
        *self == Scope::Global
    }

    /// Check if this is project scope
    pub fn is_project(&self) -> bool {
        *self == Scope::Project
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::Global => write!(f, "global"),
            Scope::Project => write!(f, "project"),
        }
    }
}

impl FromStr for Scope {
    type Err = TypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "global" | "g" | "user" => Ok(Scope::Global),
            "project" | "p" | "local" => Ok(Scope::Project),
            _ => Err(TypeError::Parse(format!("Invalid scope: {s}"))),
        }
    }
}

/// Scope filter for queries that can target one or both scopes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeFilter {
    /// Only global scope
    Global,
    /// Only project scope
    Project,
    /// Both scopes (default for search)
    #[default]
    All,
}

impl ScopeFilter {
    /// Check if this filter includes global scope
    pub fn includes_global(&self) -> bool {
        matches!(self, ScopeFilter::Global | ScopeFilter::All)
    }

    /// Check if this filter includes project scope
    pub fn includes_project(&self) -> bool {
        matches!(self, ScopeFilter::Project | ScopeFilter::All)
    }

    /// Convert to optional Scope (None means both)
    pub fn as_scope(&self) -> Option<Scope> {
        match self {
            ScopeFilter::Global => Some(Scope::Global),
            ScopeFilter::Project => Some(Scope::Project),
            ScopeFilter::All => None,
        }
    }
}

impl From<Scope> for ScopeFilter {
    fn from(scope: Scope) -> Self {
        match scope {
            Scope::Global => ScopeFilter::Global,
            Scope::Project => ScopeFilter::Project,
        }
    }
}

impl FromStr for ScopeFilter {
    type Err = TypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "global" | "g" => Ok(ScopeFilter::Global),
            "project" | "p" | "local" => Ok(ScopeFilter::Project),
            "all" | "both" | "*" => Ok(ScopeFilter::All),
            _ => Err(TypeError::Parse(format!("Invalid scope filter: {s}"))),
        }
    }
}

impl fmt::Display for ScopeFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeFilter::Global => write!(f, "global"),
            ScopeFilter::Project => write!(f, "project"),
            ScopeFilter::All => write!(f, "all"),
        }
    }
}

/// Per-entity override for who can see a memory when syncing to the cloud.
///
/// Orthogonal to `Scope` — `Scope` is *where the data lives* locally,
/// `ShareScope` is *who receives it* at sync time. See
/// `docs/requests/team-memories-filter-policy.md` for the precedence rules.
///
/// When `None` (the default), the CLI's auto-promotion policy applies:
/// Project-scoped, non-Preference entities dual-enqueue to the team push
/// queue iff a team is configured. An explicit `Private` suppresses the
/// team enqueue even for entities that would otherwise auto-promote; an
/// explicit `Team` force-promotes even Global-scoped or Preference-typed
/// entities (the server-side pull filter still applies for Preference —
/// see the filter-policy doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareScope {
    /// Never auto-promote to the team queue, regardless of scope / type.
    Private,
    /// Force-promote to the team queue, regardless of scope / type.
    Team,
}

impl fmt::Display for ShareScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShareScope::Private => write!(f, "private"),
            ShareScope::Team => write!(f, "team"),
        }
    }
}

impl FromStr for ShareScope {
    type Err = TypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "private" | "personal" => Ok(ShareScope::Private),
            "team" => Ok(ShareScope::Team),
            _ => Err(TypeError::Parse(format!("Invalid share scope: {s}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::scope::*;

    #[test]
    fn test_scope_from_str() {
        assert_eq!(Scope::from_str("global").unwrap(), Scope::Global);
        assert_eq!(Scope::from_str("GLOBAL").unwrap(), Scope::Global);
        assert_eq!(Scope::from_str("g").unwrap(), Scope::Global);
        assert_eq!(Scope::from_str("project").unwrap(), Scope::Project);
        assert_eq!(Scope::from_str("local").unwrap(), Scope::Project);
        assert!(Scope::from_str("invalid").is_err());
    }

    #[test]
    fn test_scope_display() {
        assert_eq!(Scope::Global.to_string(), "global");
        assert_eq!(Scope::Project.to_string(), "project");
    }

    #[test]
    fn test_scope_id_prefix() {
        assert_eq!(Scope::Global.id_prefix(), "g");
        assert_eq!(Scope::Project.id_prefix(), "p");
    }

    #[test]
    fn test_scope_from_id() {
        assert_eq!(Scope::from_id("g-2025-01-01-001"), Some(Scope::Global));
        assert_eq!(Scope::from_id("p-2025-01-01-001"), Some(Scope::Project));
        assert_eq!(Scope::from_id("p-cas-a1b2"), Some(Scope::Project));
        assert_eq!(Scope::from_id("g-rule-001"), Some(Scope::Global));
        // Legacy IDs without prefix
        assert_eq!(Scope::from_id("2025-01-01-001"), None);
        assert_eq!(Scope::from_id("cas-a1b2"), None);
    }

    #[test]
    fn test_scope_filter() {
        assert!(ScopeFilter::All.includes_global());
        assert!(ScopeFilter::All.includes_project());
        assert!(ScopeFilter::Global.includes_global());
        assert!(!ScopeFilter::Global.includes_project());
        assert!(!ScopeFilter::Project.includes_global());
        assert!(ScopeFilter::Project.includes_project());
    }

    #[test]
    fn test_scope_filter_from_str() {
        assert_eq!(ScopeFilter::from_str("all").unwrap(), ScopeFilter::All);
        assert_eq!(ScopeFilter::from_str("both").unwrap(), ScopeFilter::All);
        assert_eq!(
            ScopeFilter::from_str("global").unwrap(),
            ScopeFilter::Global
        );
        assert_eq!(
            ScopeFilter::from_str("project").unwrap(),
            ScopeFilter::Project
        );
    }

    #[test]
    fn test_default_scopes() {
        assert_eq!(Scope::default(), Scope::Project);
        assert_eq!(ScopeFilter::default(), ScopeFilter::All);
    }

    #[test]
    fn test_share_scope_from_str() {
        assert_eq!(ShareScope::from_str("private").unwrap(), ShareScope::Private);
        assert_eq!(ShareScope::from_str("PRIVATE").unwrap(), ShareScope::Private);
        // "personal" is a user-facing alias — the CLI speaks this word,
        // the type system stores Private.
        assert_eq!(
            ShareScope::from_str("personal").unwrap(),
            ShareScope::Private
        );
        assert_eq!(ShareScope::from_str("team").unwrap(), ShareScope::Team);
        assert!(ShareScope::from_str("public").is_err());
        assert!(ShareScope::from_str("").is_err());
    }

    #[test]
    fn test_share_scope_display() {
        assert_eq!(ShareScope::Private.to_string(), "private");
        assert_eq!(ShareScope::Team.to_string(), "team");
    }

    #[test]
    fn test_share_scope_serde_roundtrip() {
        let private_json = serde_json::to_string(&ShareScope::Private).unwrap();
        assert_eq!(private_json, r#""private""#);
        let team_json = serde_json::to_string(&ShareScope::Team).unwrap();
        assert_eq!(team_json, r#""team""#);
        assert_eq!(
            serde_json::from_str::<ShareScope>(r#""private""#).unwrap(),
            ShareScope::Private
        );
        assert_eq!(
            serde_json::from_str::<ShareScope>(r#""team""#).unwrap(),
            ShareScope::Team
        );
    }
}

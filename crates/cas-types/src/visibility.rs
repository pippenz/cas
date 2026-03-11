//! Visibility type for controlling access to shared data
//!
//! Visibility determines who can see entries, tasks, rules, and other data
//! when shared via team workspaces.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::TypeError;

/// Visibility level for shared data
///
/// Controls who can access entries, tasks, rules, and other data
/// when shared via team workspaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Only the creator can see (default)
    #[default]
    Private,

    /// All team members can see
    Team,

    /// All organization members can see
    Organization,

    /// Anyone with the link can see (future use)
    Public,
}

impl Visibility {
    /// Check if this is private visibility
    pub fn is_private(&self) -> bool {
        *self == Visibility::Private
    }

    /// Check if this is team visibility
    pub fn is_team(&self) -> bool {
        *self == Visibility::Team
    }

    /// Check if this is organization visibility
    pub fn is_organization(&self) -> bool {
        *self == Visibility::Organization
    }

    /// Check if this is public visibility
    pub fn is_public(&self) -> bool {
        *self == Visibility::Public
    }

    /// Check if this visibility allows access for team members
    pub fn allows_team_access(&self) -> bool {
        matches!(
            self,
            Visibility::Team | Visibility::Organization | Visibility::Public
        )
    }

    /// Check if this visibility allows access for organization members
    pub fn allows_org_access(&self) -> bool {
        matches!(self, Visibility::Organization | Visibility::Public)
    }
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Visibility::Private => write!(f, "private"),
            Visibility::Team => write!(f, "team"),
            Visibility::Organization => write!(f, "organization"),
            Visibility::Public => write!(f, "public"),
        }
    }
}

impl FromStr for Visibility {
    type Err = TypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "private" | "priv" => Ok(Visibility::Private),
            "team" => Ok(Visibility::Team),
            "organization" | "org" => Ok(Visibility::Organization),
            "public" | "pub" => Ok(Visibility::Public),
            _ => Err(TypeError::Parse(format!("Invalid visibility: {}", s))),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::visibility::*;

    #[test]
    fn test_visibility_default() {
        assert_eq!(Visibility::default(), Visibility::Private);
    }

    #[test]
    fn test_visibility_display() {
        assert_eq!(Visibility::Private.to_string(), "private");
        assert_eq!(Visibility::Team.to_string(), "team");
        assert_eq!(Visibility::Organization.to_string(), "organization");
        assert_eq!(Visibility::Public.to_string(), "public");
    }

    #[test]
    fn test_visibility_from_str() {
        assert_eq!(Visibility::from_str("private").unwrap(), Visibility::Private);
        assert_eq!(Visibility::from_str("priv").unwrap(), Visibility::Private);
        assert_eq!(Visibility::from_str("team").unwrap(), Visibility::Team);
        assert_eq!(Visibility::from_str("organization").unwrap(), Visibility::Organization);
        assert_eq!(Visibility::from_str("org").unwrap(), Visibility::Organization);
        assert_eq!(Visibility::from_str("public").unwrap(), Visibility::Public);
        assert_eq!(Visibility::from_str("pub").unwrap(), Visibility::Public);
        assert!(Visibility::from_str("invalid").is_err());
    }

    #[test]
    fn test_visibility_is_methods() {
        assert!(Visibility::Private.is_private());
        assert!(!Visibility::Private.is_team());
        assert!(!Visibility::Private.is_organization());
        assert!(!Visibility::Private.is_public());

        assert!(!Visibility::Team.is_private());
        assert!(Visibility::Team.is_team());

        assert!(Visibility::Organization.is_organization());
        assert!(Visibility::Public.is_public());
    }

    #[test]
    fn test_visibility_access_methods() {
        // Private allows no external access
        assert!(!Visibility::Private.allows_team_access());
        assert!(!Visibility::Private.allows_org_access());

        // Team allows team access but not org-wide
        assert!(Visibility::Team.allows_team_access());
        assert!(!Visibility::Team.allows_org_access());

        // Organization allows team and org access
        assert!(Visibility::Organization.allows_team_access());
        assert!(Visibility::Organization.allows_org_access());

        // Public allows all access
        assert!(Visibility::Public.allows_team_access());
        assert!(Visibility::Public.allows_org_access());
    }

}

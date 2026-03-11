//! Entity and relationship types for the knowledge graph
//!
//! This module defines the core types for CAS's knowledge graph feature,
//! inspired by Graphiti's temporal knowledge graph patterns.
//!
//! # Node Types (Entities)
//!
//! - **Person** - People mentioned in memories (teammates, users, etc.)
//! - **Project** - Projects, repositories, or codebases
//! - **Concept** - Abstract concepts (authentication, caching, etc.)
//! - **File** - Files and directories in the codebase
//! - **Tool** - Tools, libraries, and technologies
//! - **Organization** - Companies, teams, or groups
//!
//! # Integration Status
//! Entity extraction and graph building ready for GraphRetriever integration.

// Dead code check enabled - all items used

//! # Edge Types (Relationships)
//!
//! - **WorksOn** - Person works on project
//! - **Uses** - Entity uses tool/technology
//! - **MentionedIn** - Entity is mentioned in memory entry
//! - **RelatedTo** - General semantic relationship
//! - **PartOf** - Hierarchy/composition (file part of project)
//! - **DependsOn** - Dependency relationship
//! - **Created** - Entity created something
//! - **Modified** - Entity modified something

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::TypeError;

/// Types of entities (nodes) in the knowledge graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    /// A person (teammate, user, contributor)
    Person,
    /// A project, repository, or codebase
    Project,
    /// An abstract concept (authentication, caching, design pattern)
    #[default]
    Concept,
    /// A file or directory
    File,
    /// A tool, library, or technology
    Tool,
    /// A company, team, or organization
    Organization,
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityType::Person => write!(f, "person"),
            EntityType::Project => write!(f, "project"),
            EntityType::Concept => write!(f, "concept"),
            EntityType::File => write!(f, "file"),
            EntityType::Tool => write!(f, "tool"),
            EntityType::Organization => write!(f, "organization"),
        }
    }
}

impl FromStr for EntityType {
    type Err = TypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "person" | "user" | "contributor" => Ok(EntityType::Person),
            "project" | "repo" | "repository" | "codebase" => Ok(EntityType::Project),
            "concept" | "idea" | "pattern" => Ok(EntityType::Concept),
            "file" | "directory" | "path" => Ok(EntityType::File),
            "tool" | "library" | "technology" | "tech" | "framework" => Ok(EntityType::Tool),
            "organization" | "org" | "company" | "team" => Ok(EntityType::Organization),
            _ => Err(TypeError::Parse(format!("Invalid entity type: {s}"))),
        }
    }
}

/// Types of relationships (edges) between entities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    /// Person works on project
    WorksOn,
    /// Entity uses tool/technology
    Uses,
    /// Entity is mentioned in a memory entry
    MentionedIn,
    /// General semantic relationship
    #[default]
    RelatedTo,
    /// Hierarchy/composition (file part of project)
    PartOf,
    /// Dependency relationship
    DependsOn,
    /// Entity created something
    Created,
    /// Entity modified something
    Modified,
    /// Entity knows/is associated with another entity
    Knows,
    /// Entity owns something
    Owns,
    /// Entity implements something (interface, pattern)
    Implements,
}

impl fmt::Display for RelationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelationType::WorksOn => write!(f, "works_on"),
            RelationType::Uses => write!(f, "uses"),
            RelationType::MentionedIn => write!(f, "mentioned_in"),
            RelationType::RelatedTo => write!(f, "related_to"),
            RelationType::PartOf => write!(f, "part_of"),
            RelationType::DependsOn => write!(f, "depends_on"),
            RelationType::Created => write!(f, "created"),
            RelationType::Modified => write!(f, "modified"),
            RelationType::Knows => write!(f, "knows"),
            RelationType::Owns => write!(f, "owns"),
            RelationType::Implements => write!(f, "implements"),
        }
    }
}

impl FromStr for RelationType {
    type Err = TypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "works_on" | "workson" => Ok(RelationType::WorksOn),
            "uses" | "using" => Ok(RelationType::Uses),
            "mentioned_in" | "mentionedin" | "mentioned" => Ok(RelationType::MentionedIn),
            "related_to" | "relatedto" | "related" => Ok(RelationType::RelatedTo),
            "part_of" | "partof" | "in" | "belongs_to" => Ok(RelationType::PartOf),
            "depends_on" | "dependson" | "requires" => Ok(RelationType::DependsOn),
            "created" | "creates" => Ok(RelationType::Created),
            "modified" | "modifies" | "changed" => Ok(RelationType::Modified),
            "knows" | "associated_with" => Ok(RelationType::Knows),
            "owns" | "owned_by" => Ok(RelationType::Owns),
            "implements" | "realized_by" => Ok(RelationType::Implements),
            _ => Err(TypeError::Parse(format!("Invalid relation type: {s}"))),
        }
    }
}

/// An entity (node) in the knowledge graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Unique identifier (e.g., "ent-001" or derived from name hash)
    pub id: String,

    /// The canonical name of the entity
    pub name: String,

    /// Type of entity
    #[serde(rename = "type")]
    pub entity_type: EntityType,

    /// Alternative names/aliases for this entity
    #[serde(default)]
    pub aliases: Vec<String>,

    /// Optional description or summary
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// When the entity was first observed
    pub created: DateTime<Utc>,

    /// When the entity was last updated
    pub updated: DateTime<Utc>,

    /// Number of times this entity appears in memories
    #[serde(default)]
    pub mention_count: i32,

    /// Confidence score for this entity (0.0-1.0)
    /// Higher values indicate more certain entity identification
    #[serde(default = "default_confidence")]
    pub confidence: f32,

    /// Whether this entity is archived/inactive
    #[serde(default)]
    pub archived: bool,

    /// Optional metadata as key-value pairs
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub metadata: std::collections::HashMap<String, String>,

    /// Auto-generated summary from accumulated facts (Hindsight observation network)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,

    /// When the summary was last generated/updated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_updated: Option<DateTime<Utc>>,
}

fn default_confidence() -> f32 {
    0.8
}

impl Entity {
    /// Create a new entity with the given name and type
    pub fn new(id: String, name: String, entity_type: EntityType) -> Self {
        let now = Utc::now();
        Self {
            id,
            name,
            entity_type,
            aliases: Vec::new(),
            description: None,
            created: now,
            updated: now,
            mention_count: 1,
            confidence: default_confidence(),
            archived: false,
            metadata: std::collections::HashMap::new(),
            summary: None,
            summary_updated: None,
        }
    }

    /// Add an alias for this entity
    pub fn add_alias(&mut self, alias: String) {
        if !self.aliases.contains(&alias) && alias != self.name {
            self.aliases.push(alias);
            self.updated = Utc::now();
        }
    }

    /// Check if a name matches this entity (including aliases)
    pub fn matches_name(&self, name: &str) -> bool {
        let name_lower = name.to_lowercase();
        self.name.to_lowercase() == name_lower
            || self.aliases.iter().any(|a| a.to_lowercase() == name_lower)
    }

    /// Increment the mention count
    pub fn record_mention(&mut self) {
        self.mention_count += 1;
        self.updated = Utc::now();
    }

    /// Generate a deterministic ID from the entity name and type
    pub fn generate_id(name: &str, entity_type: EntityType) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        name.to_lowercase().hash(&mut hasher);
        entity_type.hash(&mut hasher);
        let hash = hasher.finish();

        format!("ent-{:08x}", hash as u32)
    }
}

impl Default for Entity {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: String::new(),
            name: String::new(),
            entity_type: EntityType::default(),
            aliases: Vec::new(),
            description: None,
            created: now,
            updated: now,
            mention_count: 0,
            confidence: default_confidence(),
            archived: false,
            metadata: std::collections::HashMap::new(),
            summary: None,
            summary_updated: None,
        }
    }
}

/// A relationship (edge) between two entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Unique identifier for this relationship
    pub id: String,

    /// Source entity ID
    pub source_id: String,

    /// Target entity ID
    pub target_id: String,

    /// Type of relationship
    #[serde(rename = "type")]
    pub relation_type: RelationType,

    /// When this relationship was established
    pub created: DateTime<Utc>,

    /// When this relationship became valid (for temporal graphs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<DateTime<Utc>>,

    /// When this relationship stopped being valid (for temporal graphs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<DateTime<Utc>>,

    /// Weight/strength of the relationship (0.0-1.0)
    #[serde(default = "default_weight")]
    pub weight: f32,

    /// Number of times this relationship was observed
    #[serde(default)]
    pub observation_count: i32,

    /// Optional description or context for the relationship
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Entry IDs where this relationship was observed
    #[serde(default)]
    pub source_entries: Vec<String>,
}

fn default_weight() -> f32 {
    1.0
}

impl Relationship {
    /// Create a new relationship between two entities
    pub fn new(
        id: String,
        source_id: String,
        target_id: String,
        relation_type: RelationType,
    ) -> Self {
        Self {
            id,
            source_id,
            target_id,
            relation_type,
            created: Utc::now(),
            valid_from: None,
            valid_until: None,
            weight: default_weight(),
            observation_count: 1,
            description: None,
            source_entries: Vec::new(),
        }
    }

    /// Check if this relationship is currently valid (within temporal bounds)
    pub fn is_valid(&self) -> bool {
        let now = Utc::now();

        if let Some(from) = self.valid_from {
            if now < from {
                return false;
            }
        }

        if let Some(until) = self.valid_until {
            if now > until {
                return false;
            }
        }

        true
    }

    /// Record an observation of this relationship
    pub fn record_observation(&mut self, entry_id: Option<&str>) {
        self.observation_count += 1;
        self.weight = (self.weight + 0.1).min(1.0); // Strengthen with observations

        if let Some(id) = entry_id {
            if !self.source_entries.contains(&id.to_string()) {
                self.source_entries.push(id.to_string());
            }
        }
    }

    /// Set temporal validity bounds
    pub fn set_validity(
        &mut self,
        valid_from: Option<DateTime<Utc>>,
        valid_until: Option<DateTime<Utc>>,
    ) {
        self.valid_from = valid_from;
        self.valid_until = valid_until;
    }

    /// Generate a deterministic ID from source, target, and type
    pub fn generate_id(source_id: &str, target_id: &str, relation_type: RelationType) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        source_id.hash(&mut hasher);
        target_id.hash(&mut hasher);
        relation_type.to_string().hash(&mut hasher);
        let hash = hasher.finish();

        format!("rel-{:08x}", hash as u32)
    }
}

impl Default for Relationship {
    fn default() -> Self {
        Self {
            id: String::new(),
            source_id: String::new(),
            target_id: String::new(),
            relation_type: RelationType::default(),
            created: Utc::now(),
            valid_from: None,
            valid_until: None,
            weight: default_weight(),
            observation_count: 0,
            description: None,
            source_entries: Vec::new(),
        }
    }
}

/// Link between an entity and a memory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMention {
    /// Entity ID
    pub entity_id: String,

    /// Entry ID where the entity was mentioned
    pub entry_id: String,

    /// Position in the entry content (character offset)
    #[serde(default)]
    pub position: Option<usize>,

    /// The exact text that matched the entity
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_text: Option<String>,

    /// Confidence of this mention (0.0-1.0)
    #[serde(default = "default_confidence")]
    pub confidence: f32,

    /// When this mention was recorded
    pub created: DateTime<Utc>,
}

impl EntityMention {
    /// Create a new entity mention
    pub fn new(entity_id: String, entry_id: String) -> Self {
        Self {
            entity_id,
            entry_id,
            position: None,
            matched_text: None,
            confidence: default_confidence(),
            created: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::entity::*;

    #[test]
    fn test_entity_type_from_str() {
        assert_eq!(EntityType::from_str("person").unwrap(), EntityType::Person);
        assert_eq!(
            EntityType::from_str("PROJECT").unwrap(),
            EntityType::Project
        );
        assert_eq!(EntityType::from_str("tool").unwrap(), EntityType::Tool);
        assert_eq!(EntityType::from_str("repo").unwrap(), EntityType::Project);
        assert!(EntityType::from_str("invalid").is_err());
    }

    #[test]
    fn test_relation_type_from_str() {
        assert_eq!(
            RelationType::from_str("works_on").unwrap(),
            RelationType::WorksOn
        );
        assert_eq!(RelationType::from_str("uses").unwrap(), RelationType::Uses);
        assert_eq!(
            RelationType::from_str("part_of").unwrap(),
            RelationType::PartOf
        );
        assert!(RelationType::from_str("invalid").is_err());
    }

    #[test]
    fn test_entity_creation() {
        let entity = Entity::new(
            "ent-001".to_string(),
            "Alice".to_string(),
            EntityType::Person,
        );
        assert_eq!(entity.name, "Alice");
        assert_eq!(entity.entity_type, EntityType::Person);
        assert_eq!(entity.mention_count, 1);
    }

    #[test]
    fn test_entity_aliases() {
        let mut entity = Entity::new(
            "ent-001".to_string(),
            "CAS".to_string(),
            EntityType::Project,
        );
        entity.add_alias("Coding Agent System".to_string());
        entity.add_alias("cas-project".to_string());

        assert!(entity.matches_name("CAS"));
        assert!(entity.matches_name("cas"));
        assert!(entity.matches_name("Coding Agent System"));
        assert!(!entity.matches_name("unknown"));
    }

    #[test]
    fn test_entity_id_generation() {
        let id1 = Entity::generate_id("Alice", EntityType::Person);
        let id2 = Entity::generate_id("alice", EntityType::Person);
        let id3 = Entity::generate_id("Alice", EntityType::Project);

        // Same name (case-insensitive) and type should produce same ID
        assert_eq!(id1, id2);
        // Different type should produce different ID
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_relationship_creation() {
        let rel = Relationship::new(
            "rel-001".to_string(),
            "ent-001".to_string(),
            "ent-002".to_string(),
            RelationType::WorksOn,
        );
        assert_eq!(rel.relation_type, RelationType::WorksOn);
        assert_eq!(rel.observation_count, 1);
        assert!(rel.is_valid());
    }

    #[test]
    fn test_relationship_temporal_validity() {
        use chrono::Duration;

        let mut rel = Relationship::new(
            "rel-001".to_string(),
            "ent-001".to_string(),
            "ent-002".to_string(),
            RelationType::WorksOn,
        );

        // No bounds - always valid
        assert!(rel.is_valid());

        // Set to expired period
        let past = Utc::now() - Duration::days(10);
        let yesterday = Utc::now() - Duration::days(1);
        rel.set_validity(Some(past), Some(yesterday));
        assert!(!rel.is_valid());

        // Set to future period
        let tomorrow = Utc::now() + Duration::days(1);
        let future = Utc::now() + Duration::days(10);
        rel.set_validity(Some(tomorrow), Some(future));
        assert!(!rel.is_valid());

        // Set to current period
        let past = Utc::now() - Duration::days(1);
        let future = Utc::now() + Duration::days(1);
        rel.set_validity(Some(past), Some(future));
        assert!(rel.is_valid());
    }

    #[test]
    fn test_relationship_observation() {
        let mut rel = Relationship::new(
            "rel-001".to_string(),
            "ent-001".to_string(),
            "ent-002".to_string(),
            RelationType::Uses,
        );

        // Set initial weight lower to test strengthening
        rel.weight = 0.5;
        let initial_weight = rel.weight;

        rel.record_observation(Some("entry-001"));
        rel.record_observation(Some("entry-002"));
        rel.record_observation(Some("entry-001")); // Duplicate

        assert_eq!(rel.observation_count, 4); // 1 initial + 3 calls
        assert!(rel.weight > initial_weight); // Should have increased
        assert!(rel.weight <= 1.0); // But capped at 1.0
        assert_eq!(rel.source_entries.len(), 2); // Deduped
    }
}

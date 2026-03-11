//! Sort options for CAS queries
//!
//! Provides unified sorting for entries, tasks, and search results.

use std::str::FromStr;

/// Sort order direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    /// Ascending order (oldest first, lowest first)
    Asc,
    /// Descending order (newest first, highest first)
    #[default]
    Desc,
}

impl FromStr for SortOrder {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "asc" | "ascending" => Ok(SortOrder::Asc),
            "desc" | "descending" => Ok(SortOrder::Desc),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for SortOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortOrder::Asc => write!(f, "asc"),
            SortOrder::Desc => write!(f, "desc"),
        }
    }
}

/// Sort field for entries (memories)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntrySortField {
    /// Sort by creation date
    #[default]
    Created,
    /// Sort by last update date
    Updated,
    /// Sort by importance score
    Importance,
    /// Sort by title (alphabetically)
    Title,
}

impl FromStr for EntrySortField {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "created" | "created_at" => Ok(EntrySortField::Created),
            "updated" | "updated_at" => Ok(EntrySortField::Updated),
            "importance" => Ok(EntrySortField::Importance),
            "title" | "name" => Ok(EntrySortField::Title),
            _ => Err(()),
        }
    }
}

impl EntrySortField {
    /// Get the SQL column name for this field
    pub fn sql_column(&self) -> &'static str {
        match self {
            EntrySortField::Created => "created",
            EntrySortField::Updated => "last_accessed",
            EntrySortField::Importance => "importance",
            EntrySortField::Title => "title",
        }
    }
}

/// Sort field for tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskSortField {
    /// Sort by creation date
    #[default]
    Created,
    /// Sort by last update date
    Updated,
    /// Sort by priority (0=highest priority)
    Priority,
    /// Sort by title (alphabetically)
    Title,
}

impl FromStr for TaskSortField {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "created" | "created_at" => Ok(TaskSortField::Created),
            "updated" | "updated_at" => Ok(TaskSortField::Updated),
            "priority" => Ok(TaskSortField::Priority),
            "title" | "name" => Ok(TaskSortField::Title),
            _ => Err(()),
        }
    }
}

impl TaskSortField {
    /// Get the SQL column name for this field
    pub fn sql_column(&self) -> &'static str {
        match self {
            TaskSortField::Created => "created_at",
            TaskSortField::Updated => "updated_at",
            TaskSortField::Priority => "priority",
            TaskSortField::Title => "title",
        }
    }

    /// Get the default sort order for this field
    /// (priority sorts ascending by default, dates sort descending)
    pub fn default_order(&self) -> SortOrder {
        match self {
            TaskSortField::Priority => SortOrder::Asc,
            _ => SortOrder::Desc,
        }
    }
}

/// Sort field for search results
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchSortField {
    /// Sort by relevance score (default for search)
    #[default]
    Relevance,
    /// Sort by creation date
    Created,
    /// Sort by last update date
    Updated,
}

impl FromStr for SearchSortField {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "relevance" | "score" => Ok(SearchSortField::Relevance),
            "created" | "created_at" => Ok(SearchSortField::Created),
            "updated" | "updated_at" => Ok(SearchSortField::Updated),
            _ => Err(()),
        }
    }
}

/// Sort options for entry queries
#[derive(Debug, Clone, Default)]
pub struct EntrySortOptions {
    pub field: EntrySortField,
    pub order: SortOrder,
}

impl EntrySortOptions {
    pub fn new(field: EntrySortField, order: SortOrder) -> Self {
        Self { field, order }
    }

    /// Create from optional string parameters
    pub fn from_params(sort: Option<&str>, order: Option<&str>) -> Self {
        let field = sort.and_then(|s| s.parse().ok()).unwrap_or_default();
        let order = order
            .and_then(|s| s.parse().ok())
            .unwrap_or(SortOrder::Desc);
        Self { field, order }
    }

    /// Get the SQL ORDER BY clause
    pub fn sql_order_by(&self) -> String {
        let dir = match self.order {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };
        format!("{} {}", self.field.sql_column(), dir)
    }
}

/// Sort options for task queries
#[derive(Debug, Clone, Default)]
pub struct TaskSortOptions {
    pub field: TaskSortField,
    pub order: Option<SortOrder>, // None = use field's default
}

impl TaskSortOptions {
    pub fn new(field: TaskSortField, order: Option<SortOrder>) -> Self {
        Self { field, order }
    }

    /// Create from optional string parameters
    pub fn from_params(sort: Option<&str>, order: Option<&str>) -> Self {
        let field = sort.and_then(|s| s.parse().ok()).unwrap_or_default();
        let order = order.and_then(|s| s.parse().ok());
        Self { field, order }
    }

    /// Get the effective sort order (using field default if not specified)
    pub fn effective_order(&self) -> SortOrder {
        self.order.unwrap_or_else(|| self.field.default_order())
    }

    /// Get the SQL ORDER BY clause
    pub fn sql_order_by(&self) -> String {
        let dir = match self.effective_order() {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };
        format!("{} {}", self.field.sql_column(), dir)
    }
}

/// Sort options for search queries
#[derive(Debug, Clone, Default)]
pub struct SearchSortOptions {
    pub field: SearchSortField,
    pub order: SortOrder,
}

impl SearchSortOptions {
    pub fn new(field: SearchSortField, order: SortOrder) -> Self {
        Self { field, order }
    }

    /// Create from optional string parameters
    pub fn from_params(sort: Option<&str>, order: Option<&str>) -> Self {
        let field = sort.and_then(|s| s.parse().ok()).unwrap_or_default();
        let order = order
            .and_then(|s| s.parse().ok())
            .unwrap_or(SortOrder::Desc);
        Self { field, order }
    }

    /// Check if this is a relevance sort (affects search behavior)
    pub fn is_relevance_sort(&self) -> bool {
        matches!(self.field, SearchSortField::Relevance)
    }
}

#[cfg(test)]
mod tests {
    use crate::sort::*;

    #[test]
    fn test_sort_order_from_str() {
        assert_eq!("asc".parse::<SortOrder>().unwrap(), SortOrder::Asc);
        assert_eq!("DESC".parse::<SortOrder>().unwrap(), SortOrder::Desc);
        assert_eq!("ascending".parse::<SortOrder>().unwrap(), SortOrder::Asc);
        assert!("invalid".parse::<SortOrder>().is_err());
    }

    #[test]
    fn test_entry_sort_field_from_str() {
        assert_eq!(
            "created".parse::<EntrySortField>().unwrap(),
            EntrySortField::Created
        );
        assert_eq!(
            "importance".parse::<EntrySortField>().unwrap(),
            EntrySortField::Importance
        );
        assert_eq!(
            "title".parse::<EntrySortField>().unwrap(),
            EntrySortField::Title
        );
    }

    #[test]
    fn test_task_sort_field_default_order() {
        assert_eq!(TaskSortField::Priority.default_order(), SortOrder::Asc);
        assert_eq!(TaskSortField::Created.default_order(), SortOrder::Desc);
    }

    #[test]
    fn test_entry_sort_options_sql() {
        let opts = EntrySortOptions::new(EntrySortField::Created, SortOrder::Desc);
        assert_eq!(opts.sql_order_by(), "created DESC");

        let opts = EntrySortOptions::new(EntrySortField::Importance, SortOrder::Asc);
        assert_eq!(opts.sql_order_by(), "importance ASC");
    }

    #[test]
    fn test_task_sort_options_from_params() {
        let opts = TaskSortOptions::from_params(Some("priority"), None);
        assert_eq!(opts.field, TaskSortField::Priority);
        assert_eq!(opts.effective_order(), SortOrder::Asc); // Default for priority

        let opts = TaskSortOptions::from_params(Some("created"), Some("asc"));
        assert_eq!(opts.field, TaskSortField::Created);
        assert_eq!(opts.effective_order(), SortOrder::Asc);
    }

    #[test]
    fn test_search_sort_options() {
        let opts = SearchSortOptions::from_params(None, None);
        assert!(opts.is_relevance_sort());

        let opts = SearchSortOptions::from_params(Some("created"), Some("asc"));
        assert!(!opts.is_relevance_sort());
    }
}

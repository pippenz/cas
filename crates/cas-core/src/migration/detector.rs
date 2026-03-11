//! Schema introspection for migration bootstrap detection
//!
//! Provides utilities to detect whether specific schema elements
//! (tables, columns, indexes) exist in an SQLite database.

use rusqlite::Connection;
use std::path::Path;

use crate::error::CoreError;

/// Result type for detector operations
pub type Result<T> = std::result::Result<T, CoreError>;

/// Check if a table exists in the database
pub fn table_exists(conn: &Connection, table_name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?",
        [table_name],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .unwrap_or(false)
}

/// Check if a column exists in a table
pub fn column_exists(conn: &Connection, table_name: &str, column_name: &str) -> bool {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table_name}') WHERE name = ?"),
        [column_name],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .unwrap_or(false)
}

/// Check if an index exists in the database
pub fn index_exists(conn: &Connection, index_name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name = ?",
        [index_name],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .unwrap_or(false)
}

/// Get list of all columns in a table
pub fn get_table_columns(conn: &Connection, table_name: &str) -> Vec<String> {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info('{table_name}')")) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map([], |row| row.get::<_, String>(1))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Get list of all tables in the database
pub fn get_all_tables(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
    {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map([], |row| row.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Detect which migrations have already been applied by examining schema
///
/// Returns a list of migration IDs that appear to be already applied
/// based on their detection queries.
pub fn detect_applied_migrations(cas_dir: &Path) -> Result<Vec<u32>> {
    use crate::migration::MIGRATIONS;

    let db_path = cas_dir.join("cas.db");
    if !db_path.exists() {
        return Ok(vec![]);
    }

    let conn = Connection::open(&db_path)?;
    let mut applied = Vec::new();

    for migration in MIGRATIONS.iter() {
        if let Some(detect_query) = migration.detect {
            let is_applied: i64 = conn
                .query_row(detect_query, [], |row| row.get(0))
                .unwrap_or(0);

            if is_applied > 0 {
                applied.push(migration.id);
            }
        }
    }

    Ok(applied)
}

/// Schema summary for diagnostics
#[derive(Debug, Clone)]
pub struct SchemaSummary {
    pub tables: Vec<TableInfo>,
}

/// Information about a table
#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub row_count: i64,
}

/// Get a summary of the database schema for diagnostics
pub fn get_schema_summary(cas_dir: &Path) -> Result<SchemaSummary> {
    let db_path = cas_dir.join("cas.db");
    if !db_path.exists() {
        return Ok(SchemaSummary { tables: vec![] });
    }

    let conn = Connection::open(&db_path)?;
    let table_names = get_all_tables(&conn);

    let mut tables = Vec::new();
    for name in table_names {
        let columns = get_table_columns(&conn, &name);
        let row_count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM \"{name}\""), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        tables.push(TableInfo {
            name,
            columns,
            row_count,
        });
    }

    Ok(SchemaSummary { tables })
}

#[cfg(test)]
mod tests {
    use crate::migration::detector::*;
    use tempfile::TempDir;

    #[test]
    fn test_table_exists() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("test.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute("CREATE TABLE test_table (id INTEGER PRIMARY KEY)", [])
            .unwrap();

        assert!(table_exists(&conn, "test_table"));
        assert!(!table_exists(&conn, "nonexistent"));
    }

    #[test]
    fn test_column_exists() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("test.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute(
            "CREATE TABLE test_table (id INTEGER PRIMARY KEY, name TEXT)",
            [],
        )
        .unwrap();

        assert!(column_exists(&conn, "test_table", "id"));
        assert!(column_exists(&conn, "test_table", "name"));
        assert!(!column_exists(&conn, "test_table", "nonexistent"));
    }

    #[test]
    fn test_get_table_columns() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("test.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute(
            "CREATE TABLE test_table (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)",
            [],
        )
        .unwrap();

        let columns = get_table_columns(&conn, "test_table");
        assert_eq!(columns.len(), 3);
        assert!(columns.contains(&"id".to_string()));
        assert!(columns.contains(&"name".to_string()));
        assert!(columns.contains(&"value".to_string()));
    }
}

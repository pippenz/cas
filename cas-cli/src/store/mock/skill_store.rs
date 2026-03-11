use std::collections::HashMap;
use std::sync::RwLock;

use crate::store::SkillStore;
use crate::store::mock::id_counter::IdCounter;
use crate::types::{Skill, SkillStatus};
use cas_store::{Result, StoreError};

/// In-memory mock implementation of the SkillStore trait.
#[derive(Debug)]
pub struct MockSkillStore {
    skills: RwLock<HashMap<String, Skill>>,
    id_counter: IdCounter,
    error_on_next: RwLock<Option<StoreError>>,
}

impl Default for MockSkillStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSkillStore {
    /// Create a new empty mock skill store.
    pub fn new() -> Self {
        Self {
            skills: RwLock::new(HashMap::new()),
            id_counter: IdCounter::default(),
            error_on_next: RwLock::new(None),
        }
    }

    /// Create with pre-populated skills.
    pub fn with_skills(skills: Vec<Skill>) -> Self {
        let store = Self::new();
        {
            let mut map = store.skills.write().unwrap();
            for skill in skills {
                map.insert(skill.id.clone(), skill);
            }
        }
        store
    }

    /// Inject an error.
    pub fn inject_error(&self, error: StoreError) {
        *self.error_on_next.write().unwrap() = Some(error);
    }

    fn check_error(&self) -> Result<()> {
        let mut error = self.error_on_next.write().unwrap();
        if let Some(value) = error.take() {
            return Err(value);
        }
        Ok(())
    }

    /// Get count (for testing).
    pub fn len(&self) -> usize {
        self.skills.read().unwrap().len()
    }

    /// Check if empty (for testing).
    pub fn is_empty(&self) -> bool {
        self.skills.read().unwrap().is_empty()
    }
}

impl SkillStore for MockSkillStore {
    fn init(&self) -> Result<()> {
        self.check_error()
    }

    fn generate_id(&self) -> Result<String> {
        self.check_error()?;
        let counter = self.id_counter.next();
        Ok(format!("cas-sk{counter:02x}"))
    }

    fn add(&self, skill: &Skill) -> Result<()> {
        self.check_error()?;
        let mut skills = self.skills.write().unwrap();
        if skills.contains_key(&skill.id) {
            return Err(StoreError::EntryExists(skill.id.clone()));
        }
        skills.insert(skill.id.clone(), skill.clone());
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Skill> {
        self.check_error()?;
        let skills = self.skills.read().unwrap();
        skills
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    fn update(&self, skill: &Skill) -> Result<()> {
        self.check_error()?;
        let mut skills = self.skills.write().unwrap();
        if !skills.contains_key(&skill.id) {
            return Err(StoreError::NotFound(skill.id.clone()));
        }
        skills.insert(skill.id.clone(), skill.clone());
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.check_error()?;
        let mut skills = self.skills.write().unwrap();
        skills
            .remove(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        Ok(())
    }

    fn list(&self, status: Option<SkillStatus>) -> Result<Vec<Skill>> {
        self.check_error()?;
        let skills = self.skills.read().unwrap();
        let mut list: Vec<Skill> = skills
            .values()
            .filter(|skill| status.is_none() || Some(skill.status) == status)
            .cloned()
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    fn list_enabled(&self) -> Result<Vec<Skill>> {
        self.list(Some(SkillStatus::Enabled))
    }

    fn search(&self, query: &str) -> Result<Vec<Skill>> {
        self.check_error()?;
        let skills = self.skills.read().unwrap();
        let query_lower = query.to_lowercase();
        let mut list: Vec<Skill> = skills
            .values()
            .filter(|skill| {
                skill.name.to_lowercase().contains(&query_lower)
                    || skill.description.to_lowercase().contains(&query_lower)
            })
            .cloned()
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(list)
    }

    fn close(&self) -> Result<()> {
        self.check_error()
    }
}

use std::collections::HashMap;
use std::sync::RwLock;

use crate::store::RuleStore;
use crate::store::mock::id_counter::IdCounter;
use crate::types::{Rule, RuleStatus};
use cas_store::{Result, StoreError};

/// In-memory mock implementation of the RuleStore trait.
#[derive(Debug)]
pub struct MockRuleStore {
    rules: RwLock<HashMap<String, Rule>>,
    id_counter: IdCounter,
    error_on_next: RwLock<Option<StoreError>>,
}

impl Default for MockRuleStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MockRuleStore {
    /// Create a new empty mock rule store.
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(HashMap::new()),
            id_counter: IdCounter::default(),
            error_on_next: RwLock::new(None),
        }
    }

    /// Create with pre-populated rules.
    pub fn with_rules(rules: Vec<Rule>) -> Self {
        let store = Self::new();
        {
            let mut map = store.rules.write().unwrap();
            for rule in rules {
                map.insert(rule.id.clone(), rule);
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
        self.rules.read().unwrap().len()
    }

    /// Check if empty (for testing).
    pub fn is_empty(&self) -> bool {
        self.rules.read().unwrap().is_empty()
    }
}

impl RuleStore for MockRuleStore {
    fn init(&self) -> Result<()> {
        self.check_error()
    }

    fn generate_id(&self) -> Result<String> {
        self.check_error()?;
        let counter = self.id_counter.next();
        Ok(format!("rule-{counter:03}"))
    }

    fn add(&self, rule: &Rule) -> Result<()> {
        self.check_error()?;
        let mut rules = self.rules.write().unwrap();
        if rules.contains_key(&rule.id) {
            return Err(StoreError::EntryExists(rule.id.clone()));
        }
        rules.insert(rule.id.clone(), rule.clone());
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Rule> {
        self.check_error()?;
        let rules = self.rules.read().unwrap();
        rules
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    fn update(&self, rule: &Rule) -> Result<()> {
        self.check_error()?;
        let mut rules = self.rules.write().unwrap();
        if !rules.contains_key(&rule.id) {
            return Err(StoreError::NotFound(rule.id.clone()));
        }
        rules.insert(rule.id.clone(), rule.clone());
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.check_error()?;
        let mut rules = self.rules.write().unwrap();
        rules
            .remove(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<Rule>> {
        self.check_error()?;
        let rules = self.rules.read().unwrap();
        let mut list: Vec<Rule> = rules.values().cloned().collect();
        list.sort_by(|a, b| b.created.cmp(&a.created));
        Ok(list)
    }

    fn list_proven(&self) -> Result<Vec<Rule>> {
        self.check_error()?;
        let rules = self.list()?;
        Ok(rules
            .into_iter()
            .filter(|rule| rule.status == RuleStatus::Proven)
            .collect())
    }

    fn list_critical(&self) -> Result<Vec<Rule>> {
        self.check_error()?;
        let rules = self.list()?;
        Ok(rules
            .into_iter()
            .filter(|rule| {
                rule.priority == 0
                    && (rule.status == RuleStatus::Proven || rule.status == RuleStatus::Draft)
            })
            .collect())
    }

    fn close(&self) -> Result<()> {
        self.check_error()
    }
}

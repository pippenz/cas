use crate::types::{
    Dependency, DependencyType, Entry, EntryType, Priority, Rule, RuleStatus, Skill, SkillStatus,
    Task, TaskStatus,
};

/// Create a test entry with minimal required fields.
pub fn entry(id: &str, content: &str) -> Entry {
    let mut entry = Entry::new(id.to_string(), content.to_string());
    entry.entry_type = EntryType::Learning;
    entry
}

/// Create a test entry with tags.
pub fn entry_with_tags(id: &str, content: &str, tags: Vec<&str>) -> Entry {
    let mut value = entry(id, content);
    value.tags = tags.into_iter().map(String::from).collect();
    value
}

/// Create a test rule.
pub fn rule(id: &str, content: &str) -> Rule {
    Rule::new(id.to_string(), content.to_string())
}

/// Create a proven rule.
pub fn proven_rule(id: &str, content: &str) -> Rule {
    let mut value = rule(id, content);
    value.status = RuleStatus::Proven;
    value.helpful_count = 1;
    value
}

/// Create a test task.
pub fn task(id: &str, title: &str) -> Task {
    Task::new(id.to_string(), title.to_string())
}

/// Create a task with priority.
pub fn task_with_priority(id: &str, title: &str, priority: i32) -> Task {
    let mut value = task(id, title);
    value.priority = Priority(priority);
    value
}

/// Create an in-progress task.
pub fn in_progress_task(id: &str, title: &str) -> Task {
    let mut value = task(id, title);
    value.status = TaskStatus::InProgress;
    value
}

/// Create a test skill.
pub fn skill(id: &str, name: &str) -> Skill {
    let mut value = Skill::new(id.to_string(), name.to_string());
    value.description = format!("Test skill: {name}");
    value.invocation = format!("/{}", name.to_lowercase());
    value.status = SkillStatus::Enabled;
    value
}

/// Create a disabled skill.
pub fn disabled_skill(id: &str, name: &str) -> Skill {
    let mut value = skill(id, name);
    value.status = SkillStatus::Disabled;
    value
}

/// Create a dependency.
pub fn blocks(from: &str, to: &str) -> Dependency {
    Dependency::new(from.to_string(), to.to_string(), DependencyType::Blocks)
}

//! Team auto-promotion filter policy.
//!
//! Implements the T1 decision in `docs/requests/team-memories-filter-policy.md`
//! — given an entity and (indirectly) a configured team, decide whether the
//! syncing store wrapper should dual-enqueue the write to the team push queue
//! in addition to the personal queue.
//!
//! The predicate is per-entity-type because `Entry` has a `Preference` carve-out
//! that does not apply to `Rule` / `Skill` / `Task`. Each `eligible_for_team_*`
//! function is pure — no I/O, no config reads — and is cheap enough to run on
//! every write.
//!
//! Caller-side protocol (per filter-policy Decision 1):
//!
//! 1. Resolve `cloud_config.active_team_id()` — `None` means no team configured
//!    OR `team_auto_promote = Some(false)` kill-switch. Skip team enqueue.
//! 2. Apply the per-type predicate to the entity. `false` means the rule or
//!    explicit override says "stay personal".
//! 3. Only if both pass, enqueue the team row.

use cas_types::{Entry, EntryType, Rule, Scope, ShareScope, Skill, Task};

/// Filter policy for an `Entry`.
///
/// Per filter-policy Decision 1 + Decision 2 precedence table:
///
/// | `share`   | `scope`  | `entry_type`     | result |
/// |-----------|----------|------------------|--------|
/// | `Private` | any      | any              | false  |
/// | `Team`    | any      | any              | true   |
/// | `None`    | Project  | `!= Preference`  | true   |
/// | `None`    | Project  | `Preference`     | false  |
/// | `None`    | Global   | any              | false  |
pub fn eligible_for_team_entry(entry: &Entry) -> bool {
    match entry.share {
        Some(ShareScope::Private) => false,
        Some(ShareScope::Team) => true,
        None => entry.scope == Scope::Project && entry.entry_type != EntryType::Preference,
    }
}

/// Filter policy for a `Rule`.
///
/// Rules have no `Preference` analogue. `Rule` does not carry a `share` field
/// in this release — when T5 (cas-07d7) introduces per-rule overrides, extend
/// this signature. Today the rule is: Project scope → dual-enqueue, Global
/// scope → personal-only.
pub fn eligible_for_team_rule(rule: &Rule) -> bool {
    rule.scope == Scope::Project
}

/// Filter policy for a `Skill`. Same shape as `Rule` — Project → dual,
/// Global → personal.
pub fn eligible_for_team_skill(skill: &Skill) -> bool {
    skill.scope == Scope::Project
}

/// Filter policy for a `Task`. Tasks are collaborative by nature within a
/// team-registered folder; Project-scoped tasks dual-enqueue, Global-scoped
/// (rare) do not.
pub fn eligible_for_team_task(task: &Task) -> bool {
    task.scope == Scope::Project
}

#[cfg(test)]
mod tests {
    use super::*;
    use cas_types::{BeliefType, MemoryTier, RuleCategory, RuleStatus, SkillStatus, SkillType};
    use chrono::Utc;

    fn entry_with(scope: Scope, entry_type: EntryType, share: Option<ShareScope>) -> Entry {
        Entry {
            id: "p-test-001".to_string(),
            scope,
            entry_type,
            observation_type: None,
            tags: vec![],
            created: Utc::now(),
            content: "test".to_string(),
            raw_content: None,
            compressed: false,
            memory_tier: MemoryTier::Working,
            title: None,
            helpful_count: 0,
            harmful_count: 0,
            last_accessed: None,
            archived: false,
            session_id: None,
            source_tool: None,
            pending_extraction: false,
            pending_embedding: false,
            stability: 0.5,
            access_count: 0,
            importance: 0.5,
            valid_from: None,
            valid_until: None,
            review_after: None,
            last_reviewed: None,
            domain: None,
            belief_type: BeliefType::Fact,
            confidence: 1.0,
            branch: None,
            team_id: None,
            share,
        }
    }

    // ── Entry predicate — full precedence table from filter-policy Decision 2.

    #[test]
    fn entry_share_private_blocks_promotion_regardless_of_scope_or_type() {
        let e = entry_with(Scope::Project, EntryType::Learning, Some(ShareScope::Private));
        assert!(!eligible_for_team_entry(&e));

        let e = entry_with(Scope::Global, EntryType::Learning, Some(ShareScope::Private));
        assert!(!eligible_for_team_entry(&e));

        let e = entry_with(
            Scope::Project,
            EntryType::Preference,
            Some(ShareScope::Private),
        );
        assert!(!eligible_for_team_entry(&e));
    }

    #[test]
    fn entry_share_team_forces_promotion_regardless_of_scope_or_type() {
        let e = entry_with(Scope::Global, EntryType::Preference, Some(ShareScope::Team));
        assert!(eligible_for_team_entry(&e));

        let e = entry_with(Scope::Project, EntryType::Learning, Some(ShareScope::Team));
        assert!(eligible_for_team_entry(&e));
    }

    #[test]
    fn entry_share_none_project_non_preference_auto_promotes() {
        for et in [
            EntryType::Learning,
            EntryType::Context,
            EntryType::Observation,
        ] {
            let e = entry_with(Scope::Project, et, None);
            assert!(
                eligible_for_team_entry(&e),
                "expected auto-promote for Project scope + {et:?}"
            );
        }
    }

    #[test]
    fn entry_share_none_project_preference_stays_personal() {
        let e = entry_with(Scope::Project, EntryType::Preference, None);
        assert!(!eligible_for_team_entry(&e));
    }

    #[test]
    fn entry_share_none_global_any_type_stays_personal() {
        for et in [
            EntryType::Learning,
            EntryType::Preference,
            EntryType::Context,
            EntryType::Observation,
        ] {
            let e = entry_with(Scope::Global, et, None);
            assert!(
                !eligible_for_team_entry(&e),
                "expected personal-only for Global scope + {et:?}"
            );
        }
    }

    // ── Rule / Skill / Task predicates — scope-only.

    #[test]
    fn rule_project_scope_promotes_global_does_not() {
        let mut r = Rule::default();
        r.scope = Scope::Project;
        assert!(eligible_for_team_rule(&r));

        r.scope = Scope::Global;
        assert!(!eligible_for_team_rule(&r));
    }

    #[test]
    fn skill_project_scope_promotes_global_does_not() {
        let mut s = Skill::default();
        s.scope = Scope::Project;
        assert!(eligible_for_team_skill(&s));

        s.scope = Scope::Global;
        assert!(!eligible_for_team_skill(&s));
    }

    #[test]
    fn task_project_scope_promotes_global_does_not() {
        let mut t = Task::default();
        t.scope = Scope::Project;
        assert!(eligible_for_team_task(&t));

        t.scope = Scope::Global;
        assert!(!eligible_for_team_task(&t));
    }

    // ── Suppress unused-import warnings for the imported enums used in
    //    construction helpers above.
    #[test]
    fn imported_enums_are_used() {
        let _ = BeliefType::Fact;
        let _ = MemoryTier::Working;
        let _ = RuleStatus::default();
        let _ = RuleCategory::default();
        let _ = SkillStatus::default();
        let _ = SkillType::default();
    }
}

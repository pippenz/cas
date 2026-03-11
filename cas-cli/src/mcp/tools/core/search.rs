use crate::mcp::tools::core::imports::*;

impl CasCore {
    // ========================================================================
    // Search Tools (1) - Use doc_type param for filtering
    // ========================================================================

    /// Unified search
    pub async fn cas_search(
        &self,
        Parameters(req): Parameters<SearchRequest>,
    ) -> Result<CallToolResult, McpError> {
        use crate::types::ScopeFilter;

        let search = self.open_search_index()?;

        let doc_types = req
            .doc_type
            .as_ref()
            .and_then(|t| match t.to_lowercase().as_str() {
                "entry" | "entries" | "memory" | "memories" => Some(vec![DocType::Entry]),
                "task" | "tasks" => Some(vec![DocType::Task]),
                "rule" | "rules" => Some(vec![DocType::Rule]),
                "skill" | "skills" => Some(vec![DocType::Skill]),
                "code" | "code_symbol" | "symbol" | "symbols" => Some(vec![DocType::CodeSymbol]),
                "code_file" | "file" | "files" => Some(vec![DocType::CodeFile]),
                _ => None,
            })
            .unwrap_or_default();

        // Parse scope filter
        let scope_filter: ScopeFilter = match req.scope.to_lowercase().as_str() {
            "global" => ScopeFilter::Global,
            "project" => ScopeFilter::Project,
            _ => ScopeFilter::All,
        };

        // Parse tags filter (comma-separated, case-insensitive matching)
        let tags_filter: Vec<String> = req
            .tags
            .as_ref()
            .map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Helper to check if item tags match the filter (all filter tags must be present)
        let matches_tags = |item_tags: &[String]| -> bool {
            if tags_filter.is_empty() {
                return true;
            }
            let item_tags_lower: Vec<String> = item_tags.iter().map(|t| t.to_lowercase()).collect();
            tags_filter
                .iter()
                .all(|filter_tag| item_tags_lower.iter().any(|t| t.contains(filter_tag)))
        };

        let opts = SearchOptions {
            query: req.query.clone(),
            limit: req.limit * 2, // Fetch more to account for scope filtering
            doc_types,
            ..Default::default()
        };

        let results = search.search_unified(&opts).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Search failed: {e}")),
            data: None,
        })?;

        if results.is_empty() {
            return Ok(Self::success("No results found"));
        }

        let store = self.open_store().ok();
        let task_store = self.open_task_store().ok();
        let rule_store = self.open_rule_store().ok();
        let skill_store = self.open_skill_store().ok();
        let code_store = crate::store::open_code_store(&self.cas_root).ok();

        let mut output = format!("Search results for \"{}\":\n\n", req.query);
        let mut count = 0;

        // Track seen qualified_names for code symbol deduplication
        let mut seen_qualified_names = std::collections::HashSet::new();

        for result in results.iter() {
            if count >= req.limit {
                break;
            }

            // Get preview and check scope/tags filters
            let (preview, matches_filters) = match result.doc_type {
                DocType::Entry => {
                    if let Some(ref s) = store {
                        if let Ok(e) = s.get(&result.id) {
                            let scope_ok = scope_filter == ScopeFilter::All
                                || (scope_filter == ScopeFilter::Global
                                    && e.scope == Scope::Global)
                                || (scope_filter == ScopeFilter::Project
                                    && e.scope == Scope::Project);
                            let tags_ok = matches_tags(&e.tags);
                            (format!("[Entry] {}", e.preview(60)), scope_ok && tags_ok)
                        } else {
                            (format!("[Entry] {}", result.id), tags_filter.is_empty())
                        }
                    } else {
                        (format!("[Entry] {}", result.id), tags_filter.is_empty())
                    }
                }
                DocType::Task => {
                    // Tasks are always project-scoped, have labels not tags
                    let scope_ok = scope_filter != ScopeFilter::Global;
                    // Skip tasks if tags filter specified (tasks use labels, not tags)
                    let tags_ok = tags_filter.is_empty();
                    if let Some(ref s) = task_store {
                        if let Ok(t) = s.get(&result.id) {
                            let type_label = if t.task_type == TaskType::Epic {
                                "Epic"
                            } else {
                                "Task"
                            };
                            (
                                format!(
                                    "[{}] P{} {:?} {}",
                                    type_label, t.priority.0, t.status, t.title
                                ),
                                scope_ok && tags_ok,
                            )
                        } else {
                            (format!("[Task] {}", result.id), scope_ok && tags_ok)
                        }
                    } else {
                        (format!("[Task] {}", result.id), scope_ok && tags_ok)
                    }
                }
                DocType::Rule => {
                    if let Some(ref s) = rule_store {
                        if let Ok(r) = s.get(&result.id) {
                            let scope_ok = scope_filter == ScopeFilter::All
                                || (scope_filter == ScopeFilter::Global
                                    && r.scope == Scope::Global)
                                || (scope_filter == ScopeFilter::Project
                                    && r.scope == Scope::Project);
                            let tags_ok = matches_tags(&r.tags);
                            (
                                format!("[Rule] {:?} {}", r.status, r.preview(50)),
                                scope_ok && tags_ok,
                            )
                        } else {
                            (format!("[Rule] {}", result.id), tags_filter.is_empty())
                        }
                    } else {
                        (format!("[Rule] {}", result.id), tags_filter.is_empty())
                    }
                }
                DocType::Skill => {
                    if let Some(ref s) = skill_store {
                        if let Ok(skill) = s.get(&result.id) {
                            let scope_ok = scope_filter == ScopeFilter::All
                                || (scope_filter == ScopeFilter::Global
                                    && skill.scope == Scope::Global)
                                || (scope_filter == ScopeFilter::Project
                                    && skill.scope == Scope::Project);
                            let tags_ok = matches_tags(&skill.tags);
                            (
                                format!("[Skill] {:?} {}", skill.status, skill.name),
                                scope_ok && tags_ok,
                            )
                        } else {
                            (format!("[Skill] {}", result.id), tags_filter.is_empty())
                        }
                    } else {
                        (format!("[Skill] {}", result.id), tags_filter.is_empty())
                    }
                }
                DocType::CodeSymbol => {
                    // Code symbols are project-scoped, no tags
                    let scope_ok = scope_filter != ScopeFilter::Global;
                    let tags_ok = tags_filter.is_empty();
                    if let Some(ref code_store) = code_store {
                        if let Ok(sym) = code_store.get_symbol(&result.id) {
                            // Deduplicate by qualified_name (same symbol indexed from different paths)
                            if !seen_qualified_names.insert(sym.qualified_name.clone()) {
                                continue; // Skip duplicate
                            }
                            (
                                format!(
                                    "[Code] {:?} {} in {}",
                                    sym.kind, sym.qualified_name, sym.file_path
                                ),
                                scope_ok && tags_ok,
                            )
                        } else {
                            (format!("[Code] {}", result.id), scope_ok && tags_ok)
                        }
                    } else {
                        (format!("[Code] {}", result.id), scope_ok && tags_ok)
                    }
                }
                DocType::CodeFile => {
                    // Code files are project-scoped, no tags
                    let scope_ok = scope_filter != ScopeFilter::Global;
                    let tags_ok = tags_filter.is_empty();
                    (format!("[File] {}", result.id), scope_ok && tags_ok)
                }
                DocType::Spec => {
                    // Specs are project-scoped
                    let scope_ok = scope_filter != ScopeFilter::Global;
                    let tags_ok = tags_filter.is_empty();
                    (format!("[Spec] {}", result.id), scope_ok && tags_ok)
                }
            };

            if matches_filters {
                count += 1;
                output.push_str(&format!(
                    "{}. {} (score: {:.2})\n   ID: {}\n\n",
                    count, preview, result.score, result.id
                ));
            }
        }

        if count == 0 {
            return Ok(Self::success("No results found"));
        }

        Ok(Self::success(format!("{output}Found {count} results")))
    }
}

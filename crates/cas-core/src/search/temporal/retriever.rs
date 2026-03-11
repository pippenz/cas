use crate::search::temporal::{TemporalEntryResult, TemporalQuery, TemporalRelation, TimePeriod};
use cas_types::Entry;
use chrono::{DateTime, Duration, Utc};

pub struct TemporalRetriever {
    /// Weight for creation time proximity
    pub creation_weight: f32,
    /// Weight for validity overlap
    pub validity_weight: f32,
    /// Weight for recency (recent entries score higher)
    pub recency_weight: f32,
}

impl Default for TemporalRetriever {
    fn default() -> Self {
        Self {
            creation_weight: 0.4,
            validity_weight: 0.4,
            recency_weight: 0.2,
        }
    }
}

impl TemporalRetriever {
    /// Create a new temporal retriever with custom weights
    pub fn new(creation_weight: f32, validity_weight: f32, recency_weight: f32) -> Self {
        Self {
            creation_weight,
            validity_weight,
            recency_weight,
        }
    }

    /// Retrieve entries relevant to a time period
    ///
    /// Returns entries sorted by temporal relevance score (highest first)
    pub fn retrieve(
        &self,
        entries: &[Entry],
        period: &TimePeriod,
        limit: usize,
    ) -> Vec<TemporalEntryResult> {
        let mut results: Vec<TemporalEntryResult> = entries
            .iter()
            .filter(|e| !e.archived)
            .filter_map(|e| self.score_entry(e, period))
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| {
            b.temporal_score
                .partial_cmp(&a.temporal_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        results
    }

    /// Score an entry's temporal relevance to a period
    fn score_entry(&self, entry: &Entry, period: &TimePeriod) -> Option<TemporalEntryResult> {
        let mut score = 0.0f32;
        let mut relation = TemporalRelation::PrecedesButValid;

        // Score for creation time
        if period.contains(&entry.created) {
            score += self.creation_weight;
            relation = TemporalRelation::CreatedDuring;
        } else {
            // Score based on proximity to period
            let creation_proximity = self.time_proximity(&entry.created, period);
            score += self.creation_weight * creation_proximity * 0.5;
        }

        // Score for validity overlap
        if let Some(valid_from) = entry.valid_from {
            let valid_until = entry
                .valid_until
                .unwrap_or_else(|| Utc::now() + Duration::days(36500));
            let entry_period = TimePeriod::new(valid_from, valid_until);

            if self.periods_overlap(&entry_period, period) {
                score += self.validity_weight;
                if relation != TemporalRelation::CreatedDuring {
                    relation = TemporalRelation::ValidDuring;
                }
            }
        } else {
            // No explicit validity = always valid, partial score
            score += self.validity_weight * 0.5;
        }

        // Score for recency (entries accessed recently score higher)
        if let Some(last_accessed) = entry.last_accessed {
            if period.contains(&last_accessed) {
                score += self.recency_weight;
                if relation == TemporalRelation::PrecedesButValid {
                    relation = TemporalRelation::AccessedDuring;
                }
            }
        }

        // Only include entries with some temporal relevance
        if score > 0.1 {
            Some(TemporalEntryResult {
                id: entry.id.clone(),
                temporal_score: score.min(1.0),
                temporal_relation: relation,
            })
        } else {
            None
        }
    }

    /// Calculate proximity of a timestamp to a period (0.0 = far, 1.0 = within or adjacent)
    fn time_proximity(&self, timestamp: &DateTime<Utc>, period: &TimePeriod) -> f32 {
        if period.contains(timestamp) {
            return 1.0;
        }

        // Calculate distance in days
        let distance_days = if *timestamp < period.start {
            (period.start - *timestamp).num_days().abs() as f32
        } else {
            (*timestamp - period.end).num_days().abs() as f32
        };

        // Exponential decay: 0.5^(days/30) - halves every 30 days
        0.5f32.powf(distance_days / 30.0)
    }

    /// Check if two periods overlap
    fn periods_overlap(&self, a: &TimePeriod, b: &TimePeriod) -> bool {
        a.start <= b.end && a.end >= b.start
    }

    /// Extract temporal expressions from a query string
    ///
    /// Returns (cleaned_query, temporal_period) if a temporal expression is found
    pub fn extract_temporal_query(query: &str) -> Option<(String, TimePeriod)> {
        // Common temporal prefix patterns to extract
        let temporal_prefixes = [
            "from last week",
            "from last month",
            "from today",
            "from yesterday",
            "in january",
            "in february",
            "in march",
            "in april",
            "in may",
            "in june",
            "in july",
            "in august",
            "in september",
            "in october",
            "in november",
            "in december",
            "last week",
            "last month",
            "last year",
            "this week",
            "this month",
            "this year",
            "today",
            "yesterday",
            "recently",
        ];

        let query_lower = query.to_lowercase();

        for prefix in temporal_prefixes {
            if query_lower.contains(prefix) {
                // Try to parse the temporal expression
                if let Some(temporal) = TemporalQuery::parse(prefix) {
                    // Remove the temporal expression from the query
                    let cleaned = query_lower.replace(prefix, "").trim().to_string();
                    // Also clean up connecting words
                    let cleaned = cleaned
                        .replace(" from ", " ")
                        .replace(" in ", " ")
                        .trim()
                        .to_string();

                    return Some((cleaned, temporal.period));
                }
            }
        }

        // Try to find date patterns like "2025-01-15" or "since 2025-01-01"
        let date_regex_patterns = [r"\d{4}-\d{2}-\d{2}", r"since \d{4}-\d{2}-\d{2}"];

        for pattern in date_regex_patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if let Some(m) = re.find(&query_lower) {
                    if let Some(temporal) = TemporalQuery::parse(m.as_str()) {
                        let cleaned = re.replace(&query_lower, "").trim().to_string();
                        return Some((cleaned, temporal.period));
                    }
                }
            }
        }

        // Handle "recently" specially - last 7 days
        if query_lower.contains("recently") {
            let cleaned = query_lower.replace("recently", "").trim().to_string();
            return Some((cleaned, TimePeriod::last_days(7)));
        }

        None
    }
}

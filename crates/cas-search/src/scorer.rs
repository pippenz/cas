//! Score normalization and combination utilities
//!
//! This module provides utilities for normalizing and combining search scores
//! from different sources (BM25, semantic, etc.) into unified relevance scores.
//!
//! # Score Ranges
//!
//! Different search methods produce scores in different ranges:
//! - BM25: 0 to unbounded (typically 0-20 for good matches)
//! - Cosine similarity: -1 to 1 (usually 0-1 for similar content)
//! - Combined scores: 0 to 1 (after normalization)
//!
//! # Example
//!
//! ```rust
//! use cas_search::scorer::{normalize_min_max, combine_weighted};
//!
//! let bm25_scores = vec![("a".into(), 10.0), ("b".into(), 5.0)];
//! let semantic_scores = vec![("a".into(), 0.8), ("c".into(), 0.9)];
//!
//! let bm25_norm = normalize_min_max(&bm25_scores);
//! let combined = combine_weighted(&bm25_norm, &semantic_scores, 0.4, 0.6);
//! ```

use std::collections::HashMap;

/// Normalize scores using min-max scaling to [0, 1] range
///
/// If all scores are equal, returns 1.0 for all entries.
pub fn normalize_min_max(scores: &[(String, f64)]) -> Vec<(String, f64)> {
    if scores.is_empty() {
        return Vec::new();
    }

    let min = scores.iter().map(|(_, s)| *s).fold(f64::INFINITY, f64::min);
    let max = scores
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);

    let range = max - min;

    if range < 1e-10 {
        return scores.iter().map(|(id, _)| (id.clone(), 1.0)).collect();
    }

    scores
        .iter()
        .map(|(id, score)| (id.clone(), (score - min) / range))
        .collect()
}

/// Normalize scores using percentile-based scaling
///
/// More robust to outliers than min-max normalization.
/// Scores above the percentile are soft-clipped to 1.2.
///
/// # Arguments
/// * `scores` - Input scores to normalize
/// * `percentile` - Percentile to use as the reference (e.g., 90.0 for 90th percentile)
pub fn normalize_percentile(scores: &[(String, f64)], percentile: f64) -> Vec<(String, f64)> {
    if scores.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<f64> = scores.iter().map(|(_, s)| *s).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let idx = ((percentile / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    let idx = idx.min(sorted.len() - 1);
    let p_val = sorted[idx];

    if p_val < 1e-10 {
        return scores.iter().map(|(id, _)| (id.clone(), 1.0)).collect();
    }

    scores
        .iter()
        .map(|(id, score)| {
            let normalized = (score / p_val).min(1.2);
            (id.clone(), normalized)
        })
        .collect()
}

/// Combine two score lists with weights
///
/// Documents appearing in only one list get the other score as 0.
/// Results are sorted by combined score descending.
///
/// # Arguments
/// * `scores_a` - First score list
/// * `scores_b` - Second score list
/// * `weight_a` - Weight for first list (0-1)
/// * `weight_b` - Weight for second list (0-1)
pub fn combine_weighted(
    scores_a: &[(String, f64)],
    scores_b: &[(String, f64)],
    weight_a: f64,
    weight_b: f64,
) -> Vec<(String, f64)> {
    let map_a: HashMap<&str, f64> = scores_a.iter().map(|(id, s)| (id.as_str(), *s)).collect();
    let map_b: HashMap<&str, f64> = scores_b.iter().map(|(id, s)| (id.as_str(), *s)).collect();

    let mut all_ids: Vec<&str> = map_a.keys().chain(map_b.keys()).copied().collect();
    all_ids.sort();
    all_ids.dedup();

    let total_weight = weight_a + weight_b;
    let norm_a = if total_weight > 0.0 {
        weight_a / total_weight
    } else {
        0.5
    };
    let norm_b = if total_weight > 0.0 {
        weight_b / total_weight
    } else {
        0.5
    };

    let mut results: Vec<(String, f64)> = all_ids
        .into_iter()
        .map(|id| {
            let a = map_a.get(id).copied().unwrap_or(0.0);
            let b = map_b.get(id).copied().unwrap_or(0.0);
            let combined = norm_a * a + norm_b * b;
            (id.to_string(), combined)
        })
        .collect();

    results.sort_by(|a, b| b.1.total_cmp(&a.1));
    results
}

/// Apply a multi-channel boost to documents found by multiple search methods
///
/// Documents found by multiple methods are more likely to be relevant.
///
/// # Arguments
/// * `scores` - Input scores (id, score, channel_count)
/// * `boost_per_channel` - Boost multiplier per additional channel (e.g., 0.15 = 15% boost)
pub fn apply_multi_channel_boost(
    scores: &[(String, f64, usize)],
    boost_per_channel: f64,
) -> Vec<(String, f64)> {
    scores
        .iter()
        .map(|(id, score, channels)| {
            let boost = 1.0 + boost_per_channel * (*channels as f64 - 1.0).max(0.0);
            (id.clone(), score * boost)
        })
        .collect()
}

/// Reciprocal Rank Fusion (RRF) for combining ranked lists
///
/// RRF combines rankings without requiring score normalization.
/// Each item's score is the sum of 1/(k + rank) across all rankings.
///
/// # Arguments
/// * `rankings` - Multiple ranked lists, each sorted by score descending
/// * `k` - Smoothing constant (typically 60.0)
pub fn reciprocal_rank_fusion(rankings: &[Vec<(String, f64)>], k: f64) -> Vec<(String, f64)> {
    let mut rrf_scores: HashMap<String, f64> = HashMap::new();

    for ranking in rankings {
        for (rank, (id, _)) in ranking.iter().enumerate() {
            let contribution = 1.0 / (k + rank as f64 + 1.0);
            *rrf_scores.entry(id.clone()).or_insert(0.0) += contribution;
        }
    }

    let mut results: Vec<(String, f64)> = rrf_scores.into_iter().collect();
    results.sort_by(|a, b| b.1.total_cmp(&a.1));
    results
}

/// Calibrate scores to a meaningful 0-1 range
///
/// After calibration:
/// - 0.80+ = Highly relevant
/// - 0.50-0.79 = Relevant
/// - 0.30-0.49 = Possibly relevant
/// - <0.30 = Marginal
///
/// # Arguments
/// * `scores` - Scores to calibrate (modified in place)
/// * `target_max` - Target score for the top result (default: 0.85)
pub fn calibrate(scores: &mut [(String, f64)], target_max: f64) {
    if scores.is_empty() {
        return;
    }

    let max_score = scores.iter().map(|(_, s)| *s).fold(0.0f64, f64::max);

    if max_score < 1e-10 {
        return;
    }

    let scale = target_max / max_score;

    for (_, score) in scores.iter_mut() {
        *score = (*score * scale).min(1.0);
    }
}

/// Cosine similarity between two vectors
///
/// Returns a value in [-1, 1] where:
/// - 1 = identical direction
/// - 0 = orthogonal
/// - -1 = opposite direction
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = (norm_a.sqrt() * norm_b.sqrt()).max(1e-10);
    dot / denom
}

/// Euclidean distance between two vectors
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::INFINITY;
    }

    let sum_sq: f32 = a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum();
    sum_sq.sqrt()
}

/// Convert distance to similarity (inverse relationship)
///
/// Uses the formula: similarity = 1 / (1 + distance)
pub fn distance_to_similarity(distance: f32) -> f32 {
    1.0 / (1.0 + distance)
}

#[cfg(test)]
mod tests {
    use crate::scorer::*;

    #[test]
    fn test_normalize_min_max() {
        let scores = vec![("a".into(), 10.0), ("b".into(), 5.0), ("c".into(), 0.0)];
        let norm = normalize_min_max(&scores);

        assert_eq!(norm.len(), 3);
        let a = norm.iter().find(|(id, _)| id == "a").unwrap().1;
        let c = norm.iter().find(|(id, _)| id == "c").unwrap().1;
        assert!((a - 1.0).abs() < 1e-6);
        assert!((c - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_min_max_uniform() {
        let scores = vec![("a".into(), 5.0), ("b".into(), 5.0), ("c".into(), 5.0)];
        let norm = normalize_min_max(&scores);

        for (_, score) in norm {
            assert!((score - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn test_normalize_min_max_empty() {
        let scores: Vec<(String, f64)> = vec![];
        let norm = normalize_min_max(&scores);
        assert!(norm.is_empty());
    }

    #[test]
    fn test_normalize_percentile() {
        let scores = vec![
            ("a".into(), 100.0),
            ("b".into(), 50.0),
            ("c".into(), 25.0),
            ("d".into(), 10.0),
        ];
        let norm = normalize_percentile(&scores, 90.0);

        // Top score should be >= 1.0
        let a = norm.iter().find(|(id, _)| id == "a").unwrap().1;
        assert!(a >= 1.0);

        // All scores should be <= 1.2 (soft clip)
        for (_, score) in &norm {
            assert!(*score <= 1.2);
        }
    }

    #[test]
    fn test_combine_weighted() {
        let scores_a = vec![("a".into(), 1.0), ("b".into(), 0.5)];
        let scores_b = vec![("b".into(), 0.8), ("c".into(), 0.6)];

        let combined = combine_weighted(&scores_a, &scores_b, 0.5, 0.5);

        assert_eq!(combined.len(), 3);

        // b should have highest score (appears in both)
        let b = combined.iter().find(|(id, _)| id == "b").unwrap().1;
        assert!(b > 0.0);

        // Results should be sorted by score descending
        for i in 1..combined.len() {
            assert!(combined[i - 1].1 >= combined[i].1);
        }
    }

    #[test]
    fn test_apply_multi_channel_boost() {
        let scores = vec![
            ("a".into(), 0.8, 1usize), // 1 channel
            ("b".into(), 0.6, 2),      // 2 channels
            ("c".into(), 0.5, 3),      // 3 channels
        ];

        let boosted = apply_multi_channel_boost(&scores, 0.15);

        let a = boosted.iter().find(|(id, _)| id == "a").unwrap().1;
        let b = boosted.iter().find(|(id, _)| id == "b").unwrap().1;
        let c = boosted.iter().find(|(id, _)| id == "c").unwrap().1;

        // a: 0.8 * 1.0 = 0.8
        assert!((a - 0.8).abs() < 1e-6);
        // b: 0.6 * 1.15 = 0.69
        assert!((b - 0.69).abs() < 1e-6);
        // c: 0.5 * 1.30 = 0.65
        assert!((c - 0.65).abs() < 1e-6);
    }

    #[test]
    fn test_reciprocal_rank_fusion() {
        let ranking1 = vec![("a".into(), 10.0), ("b".into(), 5.0)];
        let ranking2 = vec![("b".into(), 0.9), ("a".into(), 0.5)];

        let combined = reciprocal_rank_fusion(&[ranking1, ranking2], 60.0);

        assert_eq!(combined.len(), 2);

        // Both should have positive scores
        for (_, score) in &combined {
            assert!(*score > 0.0);
        }
    }

    #[test]
    fn test_reciprocal_rank_fusion_empty() {
        let combined = reciprocal_rank_fusion(&[], 60.0);
        assert!(combined.is_empty());
    }

    #[test]
    fn test_calibrate() {
        let mut scores = vec![("a".into(), 2.0), ("b".into(), 1.0), ("c".into(), 0.5)];

        calibrate(&mut scores, 0.85);

        // Max should be 0.85
        let max = scores.iter().map(|(_, s)| *s).fold(0.0f64, f64::max);
        assert!((max - 0.85).abs() < 0.01);

        // All should be <= 1.0
        for (_, score) in &scores {
            assert!(*score <= 1.0);
        }
    }

    #[test]
    fn test_cosine_similarity() {
        // Identical vectors
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);

        // Orthogonal vectors
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);

        // Opposite vectors
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) - (-1.0)).abs() < 1e-6);

        // Similar vectors
        let sim = cosine_similarity(&[3.0, 4.0], &[4.0, 3.0]);
        assert!(sim > 0.9);
    }

    #[test]
    fn test_cosine_similarity_edge_cases() {
        // Empty vectors
        assert_eq!(cosine_similarity(&[], &[]), 0.0);

        // Different lengths
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
    }

    #[test]
    fn test_euclidean_distance() {
        // Same point
        assert!(euclidean_distance(&[1.0, 2.0], &[1.0, 2.0]).abs() < 1e-6);

        // Known distance
        let dist = euclidean_distance(&[0.0, 0.0], &[3.0, 4.0]);
        assert!((dist - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_distance_to_similarity() {
        // Zero distance = similarity 1
        assert!((distance_to_similarity(0.0) - 1.0).abs() < 1e-6);

        // Larger distance = lower similarity
        assert!(distance_to_similarity(1.0) < distance_to_similarity(0.5));

        // Very large distance approaches 0
        assert!(distance_to_similarity(1000.0) < 0.01);
    }
}

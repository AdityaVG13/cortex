// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use super::super::*;
    fn test_jaccard_similarity_identical() {
        let score = jaccard_similarity("rust error handling", "rust error handling");
        assert!((score - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_jaccard_similarity_disjoint() {
        let score = jaccard_similarity("apple orange", "banana grape");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_jaccard_similarity_partial() {
        // "rust error" vs "rust warning" -- 1 shared ("rust"), 3 total -> 1/3
        let score = jaccard_similarity("rust error", "rust warning");
        assert!((score - 1.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_jaccard_similarity_above_threshold() {
        // "recall pipeline rrf fusion" vs "recall rrf pipeline" -- 3 shared, 4 total -> 0.75 >= 0.6
        let score = jaccard_similarity("recall pipeline rrf fusion", "recall rrf pipeline");
        assert!(score >= 0.6, "expected >= 0.6, got {score}");
    }
}

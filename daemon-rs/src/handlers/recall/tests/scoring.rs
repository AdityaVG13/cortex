// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use super::super::*;
    // ── compound scoring tests (Task 1.4) ──────────────────────────

    #[test]
    fn test_days_since() {
        let now = chrono::Utc::now();
        let today = now.to_rfc3339();
        let days_today = days_since(&today);

        // Today should be very close to 0 (within 1 minute tolerance)
        assert!(
            days_today < 0.001,
            "days_since(today) should be ~0, got {}",
            days_today
        );

        //Yesterday (approximately)
        let yesterday = (now - chrono::Duration::days(1)).to_rfc3339();
        let days_yesterday = days_since(&yesterday);
        assert!(
            (days_yesterday - 1.0).abs() < 0.02,
            "days_since(yesterday) should be ~1.0, got {}",
            days_yesterday
        );

        // Invalid timestamp should return MAX
        let days_invalid = days_since("invalid-date");
        assert_eq!(
            days_invalid,
            f64::MAX,
            "days_since(invalid) should return MAX"
        );
    }

    #[test]
    fn test_normalize() {
        // Typical range: 0-100
        assert!((normalize(0.0) - 0.0).abs() < f64::EPSILON);
        assert!((normalize(50.0) - 0.5).abs() < f64::EPSILON);
        assert!((normalize(100.0) - 1.0).abs() < f64::EPSILON);
        assert!((normalize(0.6) - 0.6).abs() < f64::EPSILON);

        // Clamp above 100
        assert_eq!(normalize(150.0), 1.0);

        // Clamp below 0
        assert_eq!(normalize(-10.0), 0.0);

        // Reject non-finite values before clamping.
        assert_eq!(normalize(f64::NAN), 0.0);
        assert_eq!(normalize(f64::INFINITY), 0.0);
    }

    #[test]
    fn test_blend_importance_uses_trust_when_available() {
        let low_trust = blend_importance(Some(0.6), Some(0.2));
        let high_trust = blend_importance(Some(0.6), Some(0.9));
        assert!(
            high_trust > low_trust,
            "higher trust should raise effective importance"
        );
        assert_eq!(
            blend_importance(Some(0.42), None),
            blend_importance(Some(0.42), Some(0.42))
        );
    }

    #[test]
    fn test_blend_importance_rejects_non_finite_values() {
        assert_eq!(blend_importance(Some(f64::NAN), None), 0.0);
        assert_eq!(
            blend_importance(Some(0.42), Some(f64::INFINITY)),
            blend_importance(Some(0.42), None)
        );
    }

    #[test]
    fn test_compound_score() {
        let now = chrono::Utc::now();
        let today = now.to_rfc3339();
        let week_ago = (now - chrono::Duration::weeks(1)).to_rfc3339();
        let month_ago = (now - chrono::Duration::days(30)).to_rfc3339();

        // High RRF, high importance, recent: should score well
        let score_high = compound_score(0.1, 100.0, &today);
        assert!(
            score_high > 0.06,
            "high RRF + high importance + recent should score well, got {}",
            score_high
        );

        // Low RRF, low importance, old: should score poorly (recency factor dominates but is low for old items)
        let score_low = compound_score(0.001, 0.0, &month_ago);
        assert!(
            score_low < 0.08,
            "low RRF + low importance + old should score poorly, got {}",
            score_low
        );

        // Recency decay: same RRF/imp, older date = lower score
        let score_today = compound_score(0.05, 50.0, &today);
        let score_week = compound_score(0.05, 50.0, &week_ago);
        assert!(
            score_today > score_week,
            "same RRF/imp, today should score > week ago"
        );
    }
}

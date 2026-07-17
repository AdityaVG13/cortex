// SPDX-License-Identifier: MIT
use super::*;

    use super::*;
    #[test]
    fn parse_duration_to_seconds_bounds_fuzzed_inputs() {
        assert_eq!(parse_duration_to_seconds("15m"), 15 * 60);
        assert_eq!(parse_duration_to_seconds("2h"), 2 * 60 * 60);
        assert_eq!(parse_duration_to_seconds("3d"), 3 * 24 * 60 * 60);
        assert_eq!(parse_duration_to_seconds("36500d"), MAX_PARSED_DURATION_SECONDS);
        for raw in [
            "",
            "m",
            "-5h",
            "10x",
            "36501d",
            "9223372036854775807m",
            "9223372036854775807h",
            "9223372036854775807d",
            "999999999999999999999999999999d",
        ] {
            assert_eq!(
                parse_duration_to_seconds(raw),
                DEFAULT_PARSED_DURATION_SECONDS,
                "duration parser should fall back for fuzzed input {raw:?}",
            );
        }
    }
    #[test]
    fn estimate_tokens_from_chars_matches_estimate_tokens() {
        for char_count in [0usize, 1, 3, 4, 38, 379, 10_000] {
            let text = "x".repeat(char_count);
            assert_eq!(
                estimate_tokens_from_chars(char_count),
                estimate_tokens(&text),
                "char-count estimator should match text estimator for {char_count} chars"
            );
        }
    }

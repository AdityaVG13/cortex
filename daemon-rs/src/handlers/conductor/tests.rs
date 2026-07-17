// SPDX-License-Identifier: MIT

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_ttl_seconds_bounds_fuzzed_request_values() {
        assert_eq!(bounded_ttl_seconds(None, 300), 300);
        assert_eq!(bounded_ttl_seconds(Some(60), 300), 60);
        assert_eq!(bounded_ttl_seconds(Some(0), 300), 1);
        assert_eq!(bounded_ttl_seconds(Some(-60), 300), 1);
        assert_eq!(
            bounded_ttl_seconds(Some(MAX_REQUEST_TTL_SECONDS + 1), 300),
            MAX_REQUEST_TTL_SECONDS
        );
        assert_eq!(
            bounded_ttl_seconds(Some(i64::MAX), SESSION_TTL_SECONDS),
            MAX_REQUEST_TTL_SECONDS
        );
    }
}

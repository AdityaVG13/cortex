// SPDX-License-Identifier: MIT
use super::*;
#[test]
fn local_bind_detection_is_strict() {
    assert!(is_local_bind_addr("127.0.0.1"));
    assert!(is_local_bind_addr("localhost"));
    assert!(is_local_bind_addr("::1"));
    assert!(!is_local_bind_addr("0.0.0.0"));
    assert!(!is_local_bind_addr("100.84.247.96"));
}
#[test]
fn plain_http_policy_rejects_team_mode_and_non_local_binds() {
    assert_eq!(
        plain_http_rejection_reason("127.0.0.1", true, false),
        Some(PlainHttpRejectionReason::TeamMode)
    );
    assert_eq!(
        plain_http_rejection_reason("0.0.0.0", false, false),
        Some(PlainHttpRejectionReason::NonLocalBind)
    );
    assert_eq!(plain_http_rejection_reason("127.0.0.1", false, false), None);
}
#[cfg(unix)]
#[test]
fn socket_activation_fd_validation_rejects_regular_file() {
    use std::os::fd::AsRawFd;
    let file = tempfile::tempfile().unwrap();
    let err = validate_socket_activation_fd(file.as_raw_fd()).unwrap_err();
    assert!(err.contains("not a socket"), "{err}");
}

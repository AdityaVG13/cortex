// SPDX-License-Identifier: MIT
use super::*;

    use super::*;
    fn test_paths(bind: &str, port: u16, ipc_endpoint: Option<&str>) -> CortexPaths {
        let temp = std::env::temp_dir().join("cortex_transport_tests");
        CortexPaths {
            home: temp.clone(),
            db: temp.join("cortex.db"),
            token: temp.join("cortex.token"),
            pid: temp.join("cortex.pid"),
            lock: temp.join("cortex.lock"),
            port,
            bind: bind.to_string(),
            ipc_endpoint: ipc_endpoint.map(|value| value.to_string()),
            models: temp.join("models"),
            write_buffer: temp.join("write_buffer.jsonl"),
        }
    }
    #[test]
    fn local_ipc_endpoint_only_resolves_for_local_targets() {
        let paths = test_paths("127.0.0.1", 7437, Some(r"\\.\pipe\cortex-daemon-7437"));
        assert_eq!(local_ipc_endpoint_for_base_url("http://127.0.0.1:7437", &paths), Some(r"\\.\pipe\cortex-daemon-7437".to_string()));
        assert_eq!(local_ipc_endpoint_for_base_url("https://api.example.com:443", &paths), None);
    }
    #[test]
    fn local_http_base_url_uses_loopback_for_wildcard_bind() {
        let paths = test_paths("0.0.0.0", 7437, None);
        assert_eq!(local_http_base_url(&paths), "http://127.0.0.1:7437");
    }
    #[test]
    fn local_http_base_url_formats_wildcard_and_ipv6_hosts() {
        assert_eq!(local_http_base_url(&test_paths("", 7437, None)), "http://127.0.0.1:7437");
        assert_eq!(local_http_base_url(&test_paths("::", 7437, None)), "http://127.0.0.1:7437");
        assert_eq!(local_http_base_url(&test_paths("[::]", 7437, None)), "http://127.0.0.1:7437");
        assert_eq!(local_http_base_url(&test_paths("::1", 7437, None)), "http://[::1]:7437");
        assert_eq!(local_http_base_url(&test_paths("[::1]", 7437, None)), "http://[::1]:7437");
    }
    #[test]
    fn parse_http_response_rejects_malformed_status_lines() {
        let malformed = [
            b"garbage 200 OK\r\n\r\n{}" as &[u8],
            b"HTTP/9.9 200 OK\r\n\r\n{}",
            b"HTTP/1.1 99 TooLow\r\n\r\n{}",
            b"HTTP/1.1 200OK\r\n\r\n{}",
            b"HTTP/1.1 abc Nope\r\n\r\n{}",
        ];
        for raw in malformed {
            assert!(parse_http_response(raw).is_err(), "malformed response should be rejected: {:?}", String::from_utf8_lossy(raw));
        }
    }

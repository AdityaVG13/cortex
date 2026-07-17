// SPDX-License-Identifier: MIT
use axum::body::Bytes;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tower::Service;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::CorsLayer;

use crate::budgets::BudgetEndpoint;
use crate::handlers;
use crate::handlers::mcp::handle_mcp_message_with_caller;
use crate::state::RuntimeState;


use super::*;
pub async fn run(
    router: Router,
    bind_addr: &str,
    port: u16,
    ipc_endpoint: Option<String>,
    db_path: &Path,
    readiness_signal: Option<Arc<AtomicBool>>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    if let Some(endpoint) = ipc_endpoint {
        spawn_ipc_listener(router.clone(), endpoint);
    }
    let mut activated_listener = match resolve_socket_activation_listener(port) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("[cortex] FATAL: socket activation listener error: {err}");
            std::process::exit(1);
        }
    };
    let policy_bind_addr = effective_bind_addr_for_policy(bind_addr, activated_listener.as_ref());

    match crate::tls::try_load_tls() {
        Ok(Some(acceptor)) => {
            run_tls(
                router,
                bind_addr,
                port,
                acceptor,
                activated_listener.take(),
                readiness_signal,
                shutdown,
            )
            .await;
        }
        Ok(None) => {
            let team_mode = detect_team_mode_for_tls(db_path);
            let allow_insecure_remote = allow_insecure_remote_http();
            if let Some(reason) =
                plain_http_rejection_reason(&policy_bind_addr, team_mode, allow_insecure_remote)
            {
                match reason {
                    PlainHttpRejectionReason::TeamMode => {
                        eprintln!("[cortex] TLS certificate not configured");
                        eprintln!(
                            "[cortex] Team mode requires valid TLS -- add certs at ~/.cortex/tls/ or set CORTEX_TLS_CERT/CORTEX_TLS_KEY"
                        );
                    }
                    PlainHttpRejectionReason::NonLocalBind => {
                        eprintln!("[cortex] TLS certificate not configured");
                        eprintln!(
                            "[cortex] Refusing plain HTTP for non-local bind '{policy_bind_addr}'."
                        );
                        eprintln!(
                            "[cortex] Add TLS certs, bind to localhost, or set CORTEX_ALLOW_INSECURE_REMOTE=1 for explicit temporary override."
                        );
                    }
                }
                std::process::exit(1);
            }
            run_plain(
                router,
                bind_addr,
                port,
                activated_listener.take(),
                readiness_signal,
                shutdown,
            )
            .await;
        }
        Err(e) => {
            // Team mode: refuse to start with broken TLS (auth integrity requires it)
            // Solo mode: allow plain fallback only for localhost binds (or explicit override).
            let team_mode = detect_team_mode_for_tls(db_path);
            let allow_insecure_remote = allow_insecure_remote_http();
            if let Some(reason) =
                plain_http_rejection_reason(&policy_bind_addr, team_mode, allow_insecure_remote)
            {
                match reason {
                    PlainHttpRejectionReason::TeamMode => {
                        eprintln!("[cortex] TLS configuration error: {e}");
                        eprintln!(
                            "[cortex] Team mode requires valid TLS -- fix certs at ~/.cortex/tls/ or set CORTEX_TLS_CERT/CORTEX_TLS_KEY"
                        );
                    }
                    PlainHttpRejectionReason::NonLocalBind => {
                        eprintln!("[cortex] TLS configuration error: {e}");
                        eprintln!(
                            "[cortex] Refusing insecure HTTP fallback for non-local bind '{policy_bind_addr}'."
                        );
                        eprintln!(
                            "[cortex] Fix TLS certs, bind to localhost, or set CORTEX_ALLOW_INSECURE_REMOTE=1 for explicit temporary override."
                        );
                    }
                }
                std::process::exit(1);
            } else {
                eprintln!("[cortex] TLS certificate error: {e}");
                if is_local_bind_addr(&policy_bind_addr) {
                    eprintln!("[cortex] Starting without TLS (solo mode -- localhost bind)");
                } else {
                    eprintln!(
                        "[cortex] Starting without TLS on non-local bind due to CORTEX_ALLOW_INSECURE_REMOTE=1"
                    );
                }
                run_plain(
                    router,
                    bind_addr,
                    port,
                    activated_listener.take(),
                    readiness_signal,
                    shutdown,
                )
                .await;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlainHttpRejectionReason {
    TeamMode,
    NonLocalBind,
}

pub(crate) fn allow_insecure_remote_http() -> bool {
    std::env::var("CORTEX_ALLOW_INSECURE_REMOTE")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

pub(crate) fn effective_bind_addr_for_policy(
    configured_bind: &str,
    activated_listener: Option<&tokio::net::TcpListener>,
) -> String {
    activated_listener
        .and_then(|listener| listener.local_addr().ok())
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| configured_bind.to_string())
}

pub(crate) fn plain_http_rejection_reason(
    bind_addr: &str,
    team_mode: bool,
    allow_insecure_remote: bool,
) -> Option<PlainHttpRejectionReason> {
    if team_mode {
        Some(PlainHttpRejectionReason::TeamMode)
    } else if !is_local_bind_addr(bind_addr) && !allow_insecure_remote {
        Some(PlainHttpRejectionReason::NonLocalBind)
    } else {
        None
    }
}

#[cfg(unix)]
pub(crate) fn resolve_socket_activation_listener(
    expected_port: u16,
) -> Result<Option<tokio::net::TcpListener>, String> {
    use std::os::fd::FromRawFd;

    pub(crate) const SYSTEMD_FIRST_SOCKET_FD: libc::c_int = 3;

    let listen_fds = std::env::var("LISTEN_FDS")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if listen_fds == 0 {
        return Ok(None);
    }

    let Some(listen_pid) = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
    else {
        return Err("LISTEN_FDS is set but LISTEN_PID is missing or invalid".to_string());
    };
    if listen_pid != std::process::id() {
        return Ok(None);
    }
    if listen_fds != 1 {
        return Err(format!(
            "Expected exactly one activated socket (LISTEN_FDS=1), got {listen_fds}"
        ));
    }

    validate_socket_activation_fd(SYSTEMD_FIRST_SOCKET_FD)?;

    // SAFETY: systemd-compatible socket activation passes the first owned
    // listening socket at fd 3 when LISTEN_PID matches this process and
    // LISTEN_FDS is exactly 1. The descriptor was also validated as open and
    // stream-socket-shaped immediately before ownership is adopted here.
    let std_listener = unsafe { std::net::TcpListener::from_raw_fd(SYSTEMD_FIRST_SOCKET_FD) };
    std_listener
        .set_nonblocking(true)
        .map_err(|e| format!("configure activated socket nonblocking: {e}"))?;
    if let Ok(addr) = std_listener.local_addr() {
        if expected_port > 0 && addr.port() != expected_port {
            eprintln!(
                "[cortex] Warning: activated socket port {} does not match configured port {}",
                addr.port(),
                expected_port
            );
        }
        eprintln!("[cortex] Using socket-activated listener on {addr}");
    } else {
        eprintln!("[cortex] Using socket-activated listener (LISTEN_FDS=1)");
    }
    std::env::remove_var("LISTEN_FDS");
    std::env::remove_var("LISTEN_PID");
    tokio::net::TcpListener::from_std(std_listener)
        .map(Some)
        .map_err(|e| format!("adopt socket-activated listener: {e}"))
}

#[cfg(unix)]
pub(crate) fn validate_socket_activation_fd(fd: libc::c_int) -> Result<(), String> {
    // SAFETY: F_GETFD only probes the integer descriptor. It does not borrow,
    // duplicate, or take ownership of the descriptor, and invalid descriptors
    // are reported as EBADF by the OS.
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
        return Err(format!(
            "activated socket fd {fd} is not open: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut socket_type: libc::c_int = 0;
    let mut socket_type_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `socket_type` and `socket_type_len` are valid out-pointers for
    // the duration of the call. `getsockopt` only observes descriptor metadata
    // and returns an error instead of taking ownership when `fd` is not a socket.
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut socket_type as *mut libc::c_int).cast(),
            &mut socket_type_len,
        )
    } < 0
    {
        return Err(format!(
            "activated socket fd {fd} is not a socket: {}",
            std::io::Error::last_os_error()
        ));
    }

    if socket_type != libc::SOCK_STREAM {
        return Err(format!(
            "activated socket fd {fd} has unsupported socket type {socket_type}; expected SOCK_STREAM"
        ));
    }

    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn resolve_socket_activation_listener(
    _expected_port: u16,
) -> Result<Option<tokio::net::TcpListener>, String> {
    Ok(None)
}

pub(crate) fn mark_runtime_ready(readiness_signal: Option<&Arc<AtomicBool>>) {
    if let Some(readiness) = readiness_signal {
        let was_ready = readiness.swap(true, Ordering::SeqCst);
        if !was_ready {
            eprintln!("[cortex] Runtime readiness gate is open");
        }
    }
}

pub(crate) fn spawn_ipc_listener(router: Router, endpoint: String) {
    tokio::spawn(async move {
        if let Err(err) = run_ipc_listener(router, endpoint.clone()).await {
            eprintln!("[cortex] IPC listener disabled for '{endpoint}': {err}");
        }
    });
}

#[cfg(unix)]
pub(crate) async fn run_ipc_listener(router: Router, endpoint: String) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    let path = std::path::PathBuf::from(&endpoint);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create IPC dir: {e}"))?;
    }
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("remove stale IPC socket: {e}"))?;
    }

    let listener = UnixListener::bind(&path).map_err(|e| format!("bind IPC socket: {e}"))?;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    eprintln!("[cortex] Listening on unix://{}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let hyper_svc = hyper_util::service::TowerToHyperService::new(router.clone());
                tokio::spawn(async move {
                    let io = hyper_util::rt::TokioIo::new(stream);
                    if let Err(err) = hyper_util::server::conn::auto::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection(io, hyper_svc)
                    .await
                    {
                        eprintln!("[cortex] IPC unix connection error: {err}");
                    }
                });
            }
            Err(err) => eprintln!("[cortex] IPC unix accept error: {err}"),
        }
    }
}

#[cfg(windows)]
pub(crate) async fn run_ipc_listener(router: Router, endpoint: String) -> Result<(), String> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut first_instance = true;
    eprintln!("[cortex] Listening on pipe://{endpoint}");
    loop {
        let mut options = ServerOptions::new();
        if first_instance {
            options.first_pipe_instance(true);
        }
        let server = match options.create(&endpoint) {
            Ok(server) => server,
            Err(err) => {
                if first_instance {
                    return Err(format!("create named pipe: {err}"));
                }
                eprintln!("[cortex] IPC pipe create error: {err}");
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                continue;
            }
        };
        first_instance = false;

        if let Err(err) = server.connect().await {
            eprintln!("[cortex] IPC pipe connect error: {err}");
            continue;
        }

        let hyper_svc = hyper_util::service::TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(server);
            if let Err(err) =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(io, hyper_svc)
                    .await
            {
                eprintln!("[cortex] IPC pipe connection error: {err}");
            }
        });
    }
}

pub(crate) fn is_local_bind_addr(bind_addr: &str) -> bool {
    matches!(
        bind_addr.trim().to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1" | "[::1]"
    )
}

/// Lightweight team-mode detection for TLS decisions (before full state init).
/// Opens the DB briefly to read the config table.
pub(crate) fn detect_team_mode_for_tls(db_path: &Path) -> bool {
    if let Ok(conn) = crate::db::open(db_path) {
        crate::db::is_team_mode(&conn)
    } else {
        false
    }
}

pub(crate) async fn run_plain(
    router: Router,
    bind_addr: &str,
    port: u16,
    activated_listener: Option<tokio::net::TcpListener>,
    readiness_signal: Option<Arc<AtomicBool>>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let listener = match activated_listener {
        Some(listener) => listener,
        None => match tokio::net::TcpListener::bind((bind_addr, port)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[cortex] FATAL: Cannot bind to {bind_addr}:{port} -- {e}");
                eprintln!("[cortex] Is another Cortex instance running? Try: cortex paths --json");
                std::process::exit(1);
            }
        },
    };
    mark_runtime_ready(readiness_signal.as_ref());
    if let Ok(addr) = listener.local_addr() {
        eprintln!("[cortex] Listening on http://{addr}");
    } else {
        eprintln!("[cortex] Listening on http://{bind_addr}:{port}");
    }
    if let Err(e) = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    {
        eprintln!("[cortex] HTTP server exited with error: {e}");
    }
}

pub(crate) async fn run_tls(
    router: Router,
    bind_addr: &str,
    port: u16,
    acceptor: tokio_rustls::TlsAcceptor,
    activated_listener: Option<tokio::net::TcpListener>,
    readiness_signal: Option<Arc<AtomicBool>>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let listener = match activated_listener {
        Some(listener) => listener,
        None => match tokio::net::TcpListener::bind((bind_addr, port)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[cortex] FATAL: Cannot bind to {bind_addr}:{port} -- {e}");
                eprintln!("[cortex] Is another Cortex instance running? Try: cortex paths --json");
                std::process::exit(1);
            }
        },
    };
    mark_runtime_ready(readiness_signal.as_ref());
    if let Ok(addr) = listener.local_addr() {
        eprintln!("[cortex] Listening on https://{addr} (TLS via rustls)");
    } else {
        eprintln!("[cortex] Listening on https://{bind_addr}:{port} (TLS via rustls)");
    }

    let mut make_svc = router.into_make_service_with_connect_info::<std::net::SocketAddr>();

    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                eprintln!("[cortex] TLS server shutting down");
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let acceptor = acceptor.clone();
                        let tower_svc = match make_svc.call(_addr).await {
                            Ok(tower_svc) => tower_svc,
                            Err(e) => {
                                eprintln!("[cortex] Failed to build TLS service for {_addr}: {e}");
                                continue;
                            }
                        };
                        tokio::spawn(async move {
                            match acceptor.accept(stream).await {
                                Ok(tls_stream) => {
                                    let hyper_svc = hyper_util::service::TowerToHyperService::new(tower_svc);
                                    let io = hyper_util::rt::TokioIo::new(tls_stream);
                                    if let Err(e) = hyper_util::server::conn::auto::Builder::new(
                                        hyper_util::rt::TokioExecutor::new(),
                                    )
                                    .serve_connection(io, hyper_svc)
                                    .await
                                    {
                                        eprintln!("[cortex] TLS connection error for {_addr}: {e}");
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[cortex] TLS handshake failed: {e}");
                                }
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("[cortex] TCP accept error: {e}");
                    }
                }
            }
        }
    }
}

pub(crate) fn parse_allowed_origin(origin: &str) -> Option<HeaderValue> {
    match origin.parse::<HeaderValue>() {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("[cortex] Invalid CORS origin '{origin}': {e}");
            None
        }
    }
}

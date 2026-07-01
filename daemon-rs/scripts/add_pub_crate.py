#!/usr/bin/env python3
"""Add pub(crate) to CLI module entry points."""

from pathlib import Path
import re

CLI = Path(__file__).resolve().parents[1] / "src" / "cli"

PRIVATE_PREFIXES = {
    "common": (
        "resolve_client_target_inputs",
        "parse_truthy_flag",
        "mask_secret_for_logs",
        "read_auth_token_from_path",
        "resolve_boot_auth_header",
    ),
    "cleanup": (
        "rotate_backups",
        "cleanup_expired_rows",
        "rotate_log_file",
        "collect_backup_cleanup_files",
        "format_cleanup_bytes",
        "path_size_bytes",
        "run_backup_cleanup",
        "run_log_cleanup",
        "run_bridge_backup_cleanup",
        "run_stale_pid_cleanup",
    ),
    "status": (
        "status_",
        "compact_status_",
        "print_status_",
        "probe_status_",
    ),
    "usage": ("top_level_command_",),
    "admin": ("print_budget_status_human",),
    "sync": (
        "validate_sync_cli_options",
        "parse_import_cli_args",
        "import_payload_from_file",
        "validate_import_",
        "validate_rfc3339",
        "resolve_sync_since",
        "read_sync_cursor",
        "write_sync_cursor",
        "ensure_sync_site",
        "sanitize_sync",
        "sync_watch_state",
        "is_sync_changeset",
        "collect_sync_watch",
        "load_sync_seen",
        "write_sync_seen",
        "acquire_sync_lock",
        "export_snapshot",
        "export_changeset",
        "write_atomic_text",
        "writable_parent",
        "sync_parent_dir",
        "run_sync_export_cli",
        "run_sync_import_cli",
        "run_sync_watch_cli",
    ),
    "daemon": (
        "daemon_lock_",
        "try_acquire_",
        "acquire_runtime",
        "daemon_owner_",
        "spawn_parent_",
        "should_watch_",
        "is_control_center",
        "parse_env_u64_nonnegative",
        "app_managed_",
        "startup_delay",
        "startup_schedule",
        "background_db_lock",
        "acquire_background_db_lock",
        "process_pid",
        "process_looks_like",
        "detect_other_cortex",
        "spawned_owner_",
        "validate_spawned_owner",
        "app_init_required",
        "local_spawn_allowed",
        "control_center_lock",
        "is_lock_contention",
        "control_center_is_active",
        "ensure_service_ready",
        "plugin_owner_tag",
        "normalized_path_for_guard",
        "path_is_under_root",
        "ensure_local_plugin_spawn",
        "read_auth_token",
        "backfill_batch",
        "collect_unembedded",
        "count_unembedded",
        "build_embeddings_async",
        "request_boot_payload",
    ),
}


def should_privatize(module: str, name: str) -> bool:
    return any(name.startswith(p) for p in PRIVATE_PREFIXES.get(module, ()))


def process_file(path: Path) -> None:
    module = path.stem
    out_lines = []
    for line in path.read_text().splitlines():
        if line.startswith("pub(crate)") or line.startswith("pub "):
            out_lines.append(line)
            continue
        m = re.match(r"^(async )?fn ([a-zA-Z0-9_]+)", line)
        if m and not should_privatize(module, m.group(2)):
            if line.startswith("async fn "):
                line = "pub(crate) " + line
            elif line.startswith("fn "):
                line = "pub(crate) " + line
        out_lines.append(line)
    path.write_text("\n".join(out_lines) + "\n")


def main() -> None:
    for path in sorted(CLI.glob("*.rs")):
        if path.name in ("mod.rs", "tests.rs"):
            continue
        process_file(path)


if __name__ == "__main__":
    main()

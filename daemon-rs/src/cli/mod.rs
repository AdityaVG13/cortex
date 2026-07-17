mod admin;
mod boot;
mod cleanup;
mod common;
mod daemon;
mod doctor;
mod embeddings;
mod eval;
mod reindex;
mod status;
mod sync;
#[cfg(test)]
mod tests;
mod usage;
pub(crate) use admin::{run_admin_cli, run_team_cli, run_user_cli};
pub(crate) use boot::run_boot_cli;
pub(crate) use cleanup::{run_backup_cli, run_cleanup_cli, run_restore_cli};
pub(crate) use common::{
    apply_path_env, ensure_remote_target_has_api_key, parse_flag_usize, parse_flag_value,
    resolve_client_target, validate_cli_options_or_exit,
};
pub(crate) use daemon::{ensure_daemon, is_disallowed_startup_binary_path, run_daemon};
pub(crate) use doctor::run_doctor_cli;
pub(crate) use embeddings::{run_embeddings_cli, run_embeddings_drain_cli};
pub(crate) use eval::run_eval_cli;
pub(crate) use reindex::{run_recrystallize_cli, run_reindex_cli};
pub(crate) use status::run_status_cli;
pub(crate) use sync::{run_export_cli, run_import_cli, run_sync_cli};
pub(crate) use usage::{
    cli_capabilities_payload, cli_capabilities_summary, cli_robot_docs_guide, cli_service_usage,
    print_usage_and_exit, unknown_cli_command_message, unknown_robot_docs_subcommand_message,
};

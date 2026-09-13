mod boot;
mod capture;
pub use capture::run_capture_cli;
mod cleanup;
mod common;
mod daemon;
mod doctor;
mod embeddings;
mod eval;
mod op;
mod reindex;
mod status;

mod usage;
pub use boot::run_boot_cli;
pub use cleanup::{run_backup_cli, run_cleanup_cli, run_restore_cli};
pub use common::{
    first_positional, parse_flag_usize, parse_flag_value, parse_flag_values, validate_cli_options_allowing_one_positional_or_exit,
    validate_cli_options_or_exit,
};
pub use daemon::run_daemon;
pub use doctor::run_doctor_cli;
pub use embeddings::{run_embeddings_cli, run_rebuild_anchors_cli};
pub use eval::run_eval_cli;
pub use op::{run_maintain_cli, run_op_cli};
pub use reindex::{run_recrystallize_cli, run_reindex_cli};
pub use status::run_status_cli;
pub use usage::{
    cli_capabilities_payload, cli_capabilities_summary, cli_robot_docs_guide, cli_service_usage, print_usage_and_exit, unknown_cli_command_message,
    unknown_robot_docs_subcommand_message,
};

// SPDX-License-Identifier: MIT
//! `cortex setup` -- Beta installer that detects AI tools and configures them.

mod types;
mod helpers;
mod team;
mod detect;
mod configure;
mod steps;

#[cfg(test)]
mod tests;

pub use types::{ConfigMethod, DetectedTool, StepResult};
pub use team::run_setup_team;
pub use steps::run_setup;

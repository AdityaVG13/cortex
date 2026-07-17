// SPDX-License-Identifier: MIT
//! `cortex setup` -- Beta installer that detects AI tools and configures them.

mod types;
mod helpers;
mod team;
mod detect;
mod configure;
mod steps;

#[cfg(test)]
mod tests {
    // Setup wizard internals are not release-gated; see Info/testing-philosophy.md.
}

pub use types::{ConfigMethod, DetectedTool, StepResult};
pub use team::run_setup_team;
pub use steps::run_setup;

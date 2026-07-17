// SPDX-License-Identifier: MIT
mod configure;
mod detect;
mod helpers;
mod steps;
mod team;
mod types;
#[cfg(test)]
mod tests {
}
pub use steps::run_setup;
pub use team::run_setup_team;

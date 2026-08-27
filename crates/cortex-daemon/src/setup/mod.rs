mod configure;
mod detect;
mod helpers;
mod steps;
mod team;
#[cfg(test)]
mod tests;
mod types;
pub use steps::run_setup;
pub use team::run_setup_team;

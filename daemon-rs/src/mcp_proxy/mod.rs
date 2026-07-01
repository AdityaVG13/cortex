// SPDX-License-Identifier: MIT
mod session;
mod run;

#[cfg(test)]
#[cfg(test)]
mod tests {
    // MCP proxy internals are not release-gated; see Info/testing-philosophy.md.
}

pub(crate) use session::*;
pub(crate) use run::*;

pub use run::run;

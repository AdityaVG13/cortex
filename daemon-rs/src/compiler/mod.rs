// SPDX-License-Identifier: MIT
mod types;
mod cache;
mod capsules;
mod ranking;
mod packing;
mod compile;

#[cfg(test)]
#[cfg(test)]
mod tests {
    // Compiler internals are not release-gated; see Info/testing-philosophy.md.
}

pub(crate) use types::*;
pub(crate) use cache::*;
pub(crate) use capsules::*;
pub(crate) use ranking::*;
pub(crate) use packing::*;
pub(crate) use compile::*;

pub use compile::compile;
pub use types::BootResult;

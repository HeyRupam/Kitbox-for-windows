//! Kitbox processing core.
//!
//! This crate is deliberately free of platform dependencies: no Tauri, no direct
//! filesystem access, no shelling out. Callers hand it bytes and options, it hands
//! back bytes and a report. That is what lets the same engine run on Windows today
//! and on Android later without a rewrite.

pub mod engine;
pub mod error;
pub mod format;
pub mod batch;
pub mod chain;
pub mod job;
pub mod preset;
pub mod quality;
pub mod size;
pub mod target;

/// Synthetic documents and images for tests, and for trying a tool by hand
/// without hunting for a file that has the right awkward property.
///
/// Behind a feature so it costs a release build nothing.
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

pub use error::{Error, Result};
pub use format::FileKind;
pub use size::{format_size, parse_size, parse_target};
pub use target::{Bias, Encodable, Outcome, SizeTarget, Solved, solve};

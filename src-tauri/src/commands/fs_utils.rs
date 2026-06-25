//! `atomic_rename` moved to `history_core::fs_utils` during the history-core
//! extraction. Re-exported here so `crate::commands::fs_utils::*` keeps
//! resolving for the many existing consumers.
pub use history_core::fs_utils::*;

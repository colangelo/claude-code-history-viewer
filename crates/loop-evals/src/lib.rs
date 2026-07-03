//! Intentionally empty. This crate exists so second-loop T2 evals
//! (`tests/<runId>_eval.rs`) have a home that can dev-depend on every
//! workspace crate — hub endpoints, history-core parsers, protocol types —
//! without distorting the dependency graph of the crates under test.

//! Turning a transcript into an action.
//!
//! The routing policy from ADR-0003 lives here. Tier 0 (`grammar`) is
//! deterministic and handles the formulaic majority of commands with no model
//! at all; Tier 1 and Tier 2 escalate from it. `execute` performs the resulting
//! command against the local store.

pub mod collections;
pub mod execute;
pub mod grammar;

pub use collections::{resolve, Candidate, Resolution};
pub use execute::{execute, execute_with, Hit, OpenTarget, Outcome};
pub use grammar::{parse, Tier0};

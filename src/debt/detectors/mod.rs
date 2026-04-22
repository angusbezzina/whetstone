//! Debt detectors. Each detector takes a project root (and, where useful,
//! the pre-walked source inventory) and returns a flat `Vec<Finding>`.
//!
//! Keep detectors:
//! - deterministic (same repo state → same findings),
//! - evidence-rich (never emit a `Finding` without a file-backed `Evidence`),
//! - cheap (walk inputs once; reuse `ast` / `detect` helpers where possible).

pub mod dead_code;
pub mod dep_hygiene;
pub mod duplicates;
pub mod hotspots;

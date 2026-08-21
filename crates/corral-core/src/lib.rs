#![forbid(unsafe_code)]

//! Corral's domain semantics and invariants: Session identity, bindings,
//! assurance, and evidence.
//!
//! This crate performs no IO and owns no wire vocabulary. Types that appear on
//! the wire live in `corral-protocol`, and surfaces depend on that crate rather
//! than on this one — a type crossing the wire does not move its business
//! semantics out of the domain (`ARCHITECTURE.md` §10).
//!
//! The domain model lands in PR2; PR1 builds the daemon skeleton around it.

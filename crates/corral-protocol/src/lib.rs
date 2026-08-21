#![forbid(unsafe_code)]

//! Corral's wire vocabulary: protocol schemas, envelopes, and the
//! compatibility-facing representations clients and daemons exchange.
//!
//! Every type here is a compatibility surface. Absent fields mean unknown,
//! never a known negative; unknown methods, notifications, fields, and
//! discriminants each have a defined behaviour; and a shipped discriminant is
//! permanent once externally released (`AGENTS.md` §Protocol).
//!
//! The hello handshake and stream vocabulary land in PR1.

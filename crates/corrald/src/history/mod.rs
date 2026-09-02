//! History enumeration: what a provider's own session store holds, by name and
//! time alone (ADR 0016 D1).

mod enumerate;

pub use enumerate::{HistoryEntry, Recent, enumerate, layout_sealed, store_root};

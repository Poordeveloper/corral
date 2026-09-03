//! History enumeration: what a provider's own session store holds, by name and
//! time alone (ADR 0016 D1), and the rows the daemon shows for what it does not
//! otherwise know (D2).

mod enumerate;
mod rows;
mod task;

pub use enumerate::{
    HistoryEntry, Recent, SealedInstall, enumerate, sealed_here, sealed_now, store_root,
};
pub use rows::{HistoryRow, HistoryRows};
pub use task::enumerate_until_shutdown;

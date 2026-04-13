//! Lee–Ready classify helpers ([`classify`]) and, with `tui`, the NBBO aggregator for the terminal UI.

pub mod classify;

#[cfg(feature = "tui")]
pub mod pipeline;

pub use classify::{lee_ready_signed_volume, price_to_row};

#[cfg(feature = "tui")]
pub use pipeline::{DashboardPipeline, DashboardSnapshot, NbboRow};

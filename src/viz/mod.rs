//! Market data visualization (Lee–Ready classify) and, with `tui`, the aggregator for terminal charts.

pub mod classify;

#[cfg(feature = "tui")]
pub mod pipeline;

pub use classify::{lee_ready_signed_volume, price_to_row};

#[cfg(feature = "tui")]
pub use pipeline::{
    DashboardPipeline, DashboardSnapshot, HEATMAP_PRICE_ROWS, HEATMAP_TIME_COLS, NBUCKETS,
    SymbolRender, WINDOW_SEC, bucket_key, bucket_ns_from_window, bucket_slot_idx,
};

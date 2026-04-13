//! Market data visualization (Lee–Ready classify) and, with `dashboard`, the aggregator + egui app.

pub mod classify;

#[cfg(feature = "dashboard")]
pub mod app;
#[cfg(feature = "dashboard")]
pub mod pipeline;
#[cfg(feature = "dashboard")]
pub mod theme;

pub use classify::{lee_ready_signed_volume, price_to_row};

#[cfg(feature = "dashboard")]
pub use pipeline::{
    bucket_key, bucket_ns_from_window, bucket_slot_idx, DashboardPipeline, DashboardSnapshot,
    SymbolRender, HEATMAP_PRICE_ROWS, HEATMAP_TIME_COLS, NBUCKETS, WINDOW_SEC,
};

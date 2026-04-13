//! Market data visualization buffers and (with `dashboard` feature) egui app.

pub mod classify;
pub mod store;

pub use classify::{lee_ready_signed_volume, price_to_row};
pub use store::{QuoteSample, SymbolSnapshot, VizStore, HEATMAP_PRICE_ROWS, HEATMAP_TIME_COLS, SERIES_CAP};

#[cfg(feature = "dashboard")]
pub mod app;

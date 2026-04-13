//! easy-rs: Alpaca market data (via [`apca`]) into [`orderbook_rs`] with a hot/cold split.
//!
//! ## Latency model (SLO-oriented)
//!
//! **End-to-end freshness** (exchange clock → local book):
//! `T_e2e ≈ T_network + T_tls + T_ws_frame + T_json + T_dispatch + T_ring + T_book`
//!
//! - **`T_book`**: time from a [`hot_path::QuoteEvent`] being visible to the consumer thread
//!   until [`orderbook_rs::DefaultOrderBook`] reflects the NBBO. Target this for microstructure
//!   research on the **local** machine; it excludes Alpaca propagation delay.
//! - **`T_ui`**: separate coalesced path for browsers (milliseconds is acceptable).
//!
//! **HotPath** (tight): bounded handoff ([`hot_path::QuoteRing`]) → single consumer applies
//! NBBO into `orderbook-rs` without logging.
//!
//! **ColdPath** (Tokio): WebSocket I/O, JSON decode, tick normalization, subscribe/resubscribe,
//! tracing, and shutdown.

pub mod cold_path;
pub mod hot_path;
pub mod orderbook_store;

pub use cold_path::symbols::SymbolRegistry;
pub use cold_path::ticks;
pub use cold_path::{AlpacaFeedConfig, AlpacaFeedSource};
pub use hot_path::quote_event::QuoteEvent;
pub use hot_path::{new_quote_ring, run_consumer};
pub use orderbook_store::BookRegistry;

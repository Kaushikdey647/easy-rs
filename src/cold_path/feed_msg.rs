//! Lock-free handoff messages from ingestion (Tokio) to the dashboard aggregator.

use crate::hot_path::quote_event::QuoteEvent;

#[derive(Clone, Copy, Debug)]
pub enum FeedMsg {
    Quote(QuoteEvent),
    Trade {
        symbol_id: u16,
        px_ticks: u64,
        sz: u64,
        ts_ns: u64,
    },
}

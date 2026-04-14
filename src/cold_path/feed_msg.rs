//! Lock-free handoff messages from ingestion (Tokio) to the aggregator (e.g. TUI).

use crate::data::OhlcvBar;
use crate::hot_path::quote_event::QuoteEvent;

#[derive(Clone, Debug)]
pub enum FeedMsg {
    Quote(QuoteEvent),
    Trade {
        symbol_id: u16,
        px_ticks: u64,
        sz: u64,
        ts_ns: u64,
    },
    /// Aggregated stream bar (when [`super::alpaca_feed::AlpacaFeedConfig::subscribe_bars`] is set).
    Bar {
        symbol_id: u16,
        ohlcv: OhlcvBar,
    },
}

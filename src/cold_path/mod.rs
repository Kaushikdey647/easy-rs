//! ColdPath: Tokio + `apca` WebSocket, JSON decode, tick conversion, fan-out to sinks.

// `feed_msg` references `crate::data::OhlcvBar` for optional bar streaming.
mod alpaca_feed;
pub mod feed_msg;
pub mod sink;
pub mod symbols;
pub mod ticks;
pub mod time_anchor;

pub use alpaca_feed::{AlpacaFeedConfig, AlpacaFeedSource, run_alpaca_quotes};
pub use feed_msg::FeedMsg;
pub use sink::AlpacaQuoteSink;

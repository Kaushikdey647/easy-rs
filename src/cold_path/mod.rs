//! ColdPath: Tokio + `apca` WebSocket, JSON decode, tick conversion, enqueue to HotPath.

mod alpaca_feed;
pub mod symbols;
pub mod ticks;

pub use alpaca_feed::{run_alpaca_quotes, AlpacaFeedConfig, AlpacaFeedSource};

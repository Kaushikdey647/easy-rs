//! Normalized OHLCV candle for [`crate::indicators`] and Alpaca bar adapters.

use apca::data::v2::bars::Bar as RestBar;
use apca::data::v2::stream::Bar as WsBar;
use chrono::{DateTime, Utc};
use num_decimal::Num;

/// Single OHLCV bar as `f64` prices (Rust side); mirrors shunya panel columns conceptually.
#[derive(Clone, Debug, PartialEq)]
pub struct OhlcvBar {
    pub time: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

fn num_f64(n: &Num) -> f64 {
    n.to_f64().unwrap_or(f64::NAN)
}

impl OhlcvBar {
    #[inline]
    pub fn vwap_proxy(&self) -> f64 {
        (self.high + self.low + self.close) / 3.0
    }
}

impl From<&RestBar> for OhlcvBar {
    fn from(b: &RestBar) -> Self {
        Self {
            time: b.time,
            open: num_f64(&b.open),
            high: num_f64(&b.high),
            low: num_f64(&b.low),
            close: num_f64(&b.close),
            volume: b.volume as f64,
        }
    }
}

impl From<&WsBar> for OhlcvBar {
    fn from(b: &WsBar) -> Self {
        Self {
            time: b.timestamp,
            open: num_f64(&b.open_price),
            high: num_f64(&b.high_price),
            low: num_f64(&b.low_price),
            close: num_f64(&b.close_price),
            volume: num_f64(&b.volume),
        }
    }
}

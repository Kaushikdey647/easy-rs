//! Fixed-size market event handed from ColdPath → HotPath.

/// Compact quote update after ColdPath normalization.
///
/// All prices are **integer ticks** at scale [`crate::cold_path::ticks::PRICE_TICK_SCALE`].
/// Sizes are whole-share counts (floored from Alpaca `Num` on the cold side).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct QuoteEvent {
    pub symbol_id: u16,
    pub bid_px_ticks: u64,
    pub ask_px_ticks: u64,
    pub bid_sz: u64,
    pub ask_sz: u64,
    /// Best-effort local monotonic-ish time in nanoseconds (from `Quote.timestamp`).
    pub ts_ns: u64,
}

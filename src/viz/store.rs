//! Per-symbol ring buffers for dashboard plots (cold-path writer, UI reader).

use crate::cold_path::ticks::PRICE_TICK_SCALE;
use crate::viz::classify::{lee_ready_signed_volume, price_to_row};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

/// Exchange timestamp (seconds) and derived plot fields for one quote.
#[derive(Clone, Copy, Debug)]
pub struct QuoteSample {
    pub exch_ts_sec: f64,
    pub bid: f64,
    pub ask: f64,
    pub spread: f64,
    pub latency_ms: f32,
    pub cum_trade_delta: i64,
}

pub const HEATMAP_TIME_COLS: usize = 256;
pub const HEATMAP_PRICE_ROWS: usize = 128;
pub const SERIES_CAP: usize = 4096;

struct SymbolState {
    samples: Vec<QuoteSample>,
    /// Ring head: samples.len() capped, drop oldest by truncating front when over cap
    samples_overflow: usize,
    /// `(exchange_time_sec, cumulative signed volume)` at each classified trade.
    delta_series: Vec<(f64, i64)>,
    last_bid_ticks: u64,
    last_ask_ticks: u64,
    have_nbbo: bool,
    last_trade_px: Option<u64>,
    cum_delta: i64,
    heatmap: Vec<f32>,
    heatmap_seq: u64,
    price_lo: f64,
    price_hi: f64,
}

impl SymbolState {
    fn new() -> Self {
        Self {
            samples: Vec::with_capacity(SERIES_CAP.min(512)),
            samples_overflow: 0,
            delta_series: Vec::with_capacity(256),
            last_bid_ticks: 0,
            last_ask_ticks: 0,
            have_nbbo: false,
            last_trade_px: None,
            cum_delta: 0,
            heatmap: vec![0.0f32; HEATMAP_TIME_COLS * HEATMAP_PRICE_ROWS],
            heatmap_seq: 0,
            price_lo: f64::MAX,
            price_hi: f64::MIN,
        }
    }

    fn ticks_to_f64(px_ticks: u64) -> f64 {
        px_ticks as f64 / PRICE_TICK_SCALE as f64
    }

    fn push_sample(&mut self, s: QuoteSample) {
        if self.samples.len() >= SERIES_CAP {
            let drop = self.samples.len() - SERIES_CAP + 1;
            self.samples.drain(0..drop);
            self.samples_overflow = self.samples_overflow.saturating_add(drop);
        }
        self.samples.push(s);
    }

    fn update_heatmap(
        &mut self,
        bid_px: f64,
        ask_px: f64,
        bid_sz: f64,
        ask_sz: f64,
        focus: bool,
    ) {
        if !focus {
            return;
        }
        self.price_lo = self.price_lo.min(bid_px).min(ask_px);
        self.price_hi = self.price_hi.max(bid_px).max(ask_px);
        if !self.price_lo.is_finite() || !self.price_hi.is_finite() {
            return;
        }
        if (self.price_hi - self.price_lo) < 1e-9 {
            self.price_hi = self.price_lo + 1e-6;
        }

        self.heatmap_seq = self.heatmap_seq.wrapping_add(1);
        let col = (self.heatmap_seq as usize) % HEATMAP_TIME_COLS;
        for row in 0..HEATMAP_PRICE_ROWS {
            self.heatmap[row * HEATMAP_TIME_COLS + col] = 0.0;
        }

        let br = price_to_row(bid_px, self.price_lo, self.price_hi, HEATMAP_PRICE_ROWS);
        let ar = price_to_row(ask_px, self.price_lo, self.price_hi, HEATMAP_PRICE_ROWS);
        let v_bid = (1.0 + bid_sz.abs()).ln() as f32;
        let v_ask = (1.0 + ask_sz.abs()).ln() as f32;
        self.heatmap[br * HEATMAP_TIME_COLS + col] = v_bid;
        self.heatmap[ar * HEATMAP_TIME_COLS + col] = v_ask;
    }
}

/// Shared visualization state updated from the Alpaca cold path.
pub struct VizStore {
    symbols: Vec<Mutex<SymbolState>>,
    /// Only this `symbol_id` gets heatmap column updates (set from UI).
    pub heatmap_focus_symbol: AtomicU16,
}

impl VizStore {
    pub fn new(symbol_count: usize) -> Arc<Self> {
        let symbols = (0..symbol_count).map(|_| Mutex::new(SymbolState::new())).collect();
        Arc::new(Self {
            symbols,
            heatmap_focus_symbol: AtomicU16::new(0),
        })
    }

    #[inline]
    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    /// Record a quote (after ring push). `exch_ts_ns` is Alpaca quote time; `local_rx_ns` wall clock for latency.
    pub fn on_quote(
        &self,
        symbol_id: u16,
        bid_px_ticks: u64,
        ask_px_ticks: u64,
        bid_sz: u64,
        ask_sz: u64,
        exch_ts_ns: u64,
        local_rx_ns: u64,
    ) {
        let idx = symbol_id as usize;
        if idx >= self.symbols.len() {
            return;
        }
        let focus = self.heatmap_focus_symbol.load(Ordering::Relaxed) == symbol_id;
        let mut g = self.symbols[idx].lock();
        g.last_bid_ticks = bid_px_ticks;
        g.last_ask_ticks = ask_px_ticks;
        g.have_nbbo = true;

        let bid = SymbolState::ticks_to_f64(bid_px_ticks);
        let ask = SymbolState::ticks_to_f64(ask_px_ticks);
        let exch_ts_sec = exch_ts_ns as f64 / 1_000_000_000.0;
        let lat_ns = local_rx_ns.saturating_sub(exch_ts_ns);
        let latency_ms = (lat_ns as f64 / 1_000_000.0).clamp(-9999.0, 9999.0) as f32;
        let cum_trade_delta = g.cum_delta;

        g.push_sample(QuoteSample {
            exch_ts_sec: exch_ts_sec,
            bid,
            ask,
            spread: ask - bid,
            latency_ms,
            cum_trade_delta,
        });

        g.update_heatmap(
            bid,
            ask,
            bid_sz as f64,
            ask_sz as f64,
            focus,
        );
    }

    /// Classify trade with last NBBO for this symbol; updates cumulative delta.
    pub fn on_trade(
        &self,
        symbol_id: u16,
        trade_px_ticks: u64,
        trade_sz: u64,
        _exch_ts_ns: u64,
    ) {
        let idx = symbol_id as usize;
        if idx >= self.symbols.len() {
            return;
        }
        let mut g = self.symbols[idx].lock();
        if !g.have_nbbo {
            return;
        }
        let (signed, last_px) = lee_ready_signed_volume(
            trade_px_ticks,
            trade_sz,
            g.last_bid_ticks,
            g.last_ask_ticks,
            g.last_trade_px,
        );
        if let Some(p) = last_px {
            g.last_trade_px = Some(p);
        }
        g.cum_delta = g.cum_delta.saturating_add(signed);
        if signed != 0 {
            let exch_ts_sec = _exch_ts_ns as f64 / 1_000_000_000.0;
            let cum = g.cum_delta;
            g.delta_series.push((exch_ts_sec, cum));
            if g.delta_series.len() > SERIES_CAP {
                let drop = g.delta_series.len() - SERIES_CAP;
                g.delta_series.drain(0..drop);
            }
        }
    }

    /// Snapshot for UI (short lock).
    pub fn snapshot_symbol(&self, symbol_id: u16) -> SymbolSnapshot {
        let idx = symbol_id as usize;
        if idx >= self.symbols.len() {
            return SymbolSnapshot::default();
        }
        let g = self.symbols[idx].lock();
        SymbolSnapshot {
            samples: g.samples.clone(),
            delta_series: g.delta_series.clone(),
            heatmap: g.heatmap.clone(),
            price_lo: g.price_lo,
            price_hi: g.price_hi,
            cum_delta: g.cum_delta,
        }
    }
}

#[derive(Clone, Default)]
pub struct SymbolSnapshot {
    pub samples: Vec<QuoteSample>,
    pub delta_series: Vec<(f64, i64)>,
    pub heatmap: Vec<f32>,
    pub price_lo: f64,
    pub price_hi: f64,
    pub cum_delta: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_latency_ms_positive_small() {
        let s = VizStore::new(1);
        let exch = 1_000_000_000u64;
        let local = exch + 1_500_000; // +1.5 ms
        s.on_quote(0, 100, 200, 1, 1, exch, local);
        let snap = s.snapshot_symbol(0);
        assert_eq!(snap.samples.len(), 1);
        assert!(snap.samples[0].latency_ms >= 1.0 && snap.samples[0].latency_ms <= 3.0);
    }

    #[test]
    fn trade_updates_delta_series() {
        let s = VizStore::new(1);
        s.on_quote(0, 100, 200, 1, 1, 1_000_000_000, 1_000_000_100);
        s.on_trade(0, 180, 5, 1_000_000_200);
        let snap = s.snapshot_symbol(0);
        assert!(!snap.delta_series.is_empty());
    }
}

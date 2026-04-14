//! Aggregator thread: pops [`FeedMsg`](crate::cold_path::FeedMsg), merges **quotes** (NBBO) and
//! **trades** (last print) per symbol, publishes [`DashboardSnapshot`] via [`arc_swap::ArcSwap`].

use crate::cold_path::feed_msg::FeedMsg;
use crate::cold_path::ticks::PRICE_TICK_SCALE;
use crate::hot_path::quote_event::QuoteEvent;
use arc_swap::ArcSwap;
use crossbeam_queue::ArrayQueue;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

const AGG_BATCH: usize = 256;

/// Latest merged quote + last trade for one symbol (display prices in dollars).
#[derive(Clone, Debug, Default)]
pub struct NbboRow {
    /// At least one NBBO quote has been merged.
    pub valid: bool,
    pub bid: f64,
    pub ask: f64,
    pub bid_sz: u64,
    pub ask_sz: u64,
    /// `ask - bid` in dollars.
    pub spread_abs: f64,
    /// `100 * (ask - bid) / mid` when `mid = (bid+ask)/2` is finite and sufficiently non-zero; else NaN.
    pub spread_pct: f64,
    pub lat_ms: f32,
    /// Last trade price (dollars); preserved across quote updates.
    pub last_px: f64,
    pub last_sz: u64,
}

#[derive(Clone, Default)]
pub struct DashboardSnapshot {
    pub symbols: Vec<NbboRow>,
}

pub struct DashboardPipeline {
    pub snapshot: Arc<ArcSwap<DashboardSnapshot>>,
    pub feed_dropped: Arc<AtomicU64>,
}

impl DashboardPipeline {
    pub fn new(symbol_count: usize, feed_dropped: Arc<AtomicU64>) -> Arc<Self> {
        let snap = DashboardSnapshot {
            symbols: (0..symbol_count).map(|_| NbboRow::default()).collect(),
        };
        Arc::new(Self {
            snapshot: Arc::new(ArcSwap::from_pointee(snap)),
            feed_dropped,
        })
    }

    pub fn spawn_aggregator(
        self: &Arc<Self>,
        queue: Arc<ArrayQueue<FeedMsg>>,
        shutdown: Arc<AtomicBool>,
    ) {
        let me = Arc::clone(self);
        let q = Arc::clone(&queue);
        let sd = Arc::clone(&shutdown);
        std::thread::spawn(move || aggregator_loop(me, q, sd));
    }
}

#[inline]
fn ticks_to_f64(px_ticks: u64) -> f64 {
    px_ticks as f64 / PRICE_TICK_SCALE as f64
}

fn merge_quote(states: &mut [NbboRow], ev: QuoteEvent) {
    let id = ev.symbol_id as usize;
    if id >= states.len() {
        return;
    }
    let prev = states[id].clone();
    let bid = ticks_to_f64(ev.bid_px_ticks);
    let ask = ticks_to_f64(ev.ask_px_ticks);
    let spread_abs = ask - bid;
    let mid = (bid + ask) / 2.0;
    let spread_pct = if mid.is_finite() && mid.abs() > 1e-12 {
        100.0 * spread_abs / mid
    } else {
        f64::NAN
    };
    let lat_ns = ev.local_rx_ns.saturating_sub(ev.ts_ns);
    let lat_ms = (lat_ns as f64 / 1_000_000.0).clamp(-9999.0, 9999.0) as f32;

    states[id] = NbboRow {
        valid: true,
        bid,
        ask,
        bid_sz: ev.bid_sz,
        ask_sz: ev.ask_sz,
        spread_abs,
        spread_pct,
        lat_ms,
        last_px: prev.last_px,
        last_sz: prev.last_sz,
    };
}

fn merge_trade(states: &mut [NbboRow], symbol_id: u16, px_ticks: u64, sz: u64) {
    let id = symbol_id as usize;
    if id >= states.len() {
        return;
    }
    let prev = states[id].clone();
    states[id] = NbboRow {
        last_px: ticks_to_f64(px_ticks),
        last_sz: sz,
        ..prev
    };
}

fn apply_feed_msg(states: &mut [NbboRow], msg: FeedMsg) {
    match msg {
        FeedMsg::Quote(ev) => merge_quote(states, ev),
        FeedMsg::Trade {
            symbol_id,
            px_ticks,
            sz,
            ..
        } => merge_trade(states, symbol_id, px_ticks, sz),
        FeedMsg::Bar { .. } => {}
    }
}

fn build_snapshot(states: &[NbboRow]) -> DashboardSnapshot {
    DashboardSnapshot {
        symbols: states.to_vec(),
    }
}

fn aggregator_loop(
    pipe: Arc<DashboardPipeline>,
    queue: Arc<ArrayQueue<FeedMsg>>,
    shutdown: Arc<AtomicBool>,
) {
    let n_sym = pipe.snapshot.load().symbols.len();
    let mut states: Vec<NbboRow> = (0..n_sym).map(|_| NbboRow::default()).collect();

    while !shutdown.load(Ordering::Relaxed) {
        let mut did = 0usize;
        while did < AGG_BATCH {
            match queue.pop() {
                Some(msg) => {
                    apply_feed_msg(&mut states, msg);
                    did += 1;
                }
                None => break,
            }
        }

        if did == 0 {
            std::thread::sleep(Duration::from_micros(50));
            continue;
        }

        let snap = build_snapshot(&states);
        pipe.snapshot.store(Arc::new(snap));
    }

    let snap = build_snapshot(&states);
    pipe.snapshot.store(Arc::new(snap));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_quote(bid_ticks: u64, ask_ticks: u64) -> QuoteEvent {
        let exch = 1_000_000_000u64;
        let local = exch + 2_000_000;
        QuoteEvent {
            symbol_id: 0,
            bid_px_ticks: bid_ticks,
            ask_px_ticks: ask_ticks,
            bid_sz: 100,
            ask_sz: 200,
            ts_ns: exch,
            local_rx_ns: local,
            ingest_mono_ns: 0,
        }
    }

    #[test]
    fn quote_sets_spread_and_pct() {
        let mut states = vec![NbboRow::default()];
        let scale = PRICE_TICK_SCALE as f64;
        merge_quote(&mut states, sample_quote(
            (10.0 * scale) as u64,
            (10.02 * scale) as u64,
        ));
        let r = &states[0];
        assert!(r.valid);
        assert!((r.bid - 10.0).abs() < 1e-6);
        assert!((r.ask - 10.02).abs() < 1e-6);
        assert!((r.spread_abs - 0.02).abs() < 1e-6);
        assert!((r.spread_pct - 100.0 * 0.02 / 10.01).abs() < 1e-4);
        assert!((f64::from(r.lat_ms) - 2.0).abs() < 0.01);
        assert_eq!(r.bid_sz, 100);
        assert_eq!(r.ask_sz, 200);
    }

    #[test]
    fn trade_then_quote_preserves_each() {
        let mut states = vec![NbboRow::default()];
        let scale = PRICE_TICK_SCALE as f64;
        merge_trade(&mut states, 0, (10.05 * scale) as u64, 42);
        assert!(!states[0].valid);
        assert_eq!(states[0].last_sz, 42);
        merge_quote(&mut states, sample_quote(
            (10.0 * scale) as u64,
            (10.02 * scale) as u64,
        ));
        assert!(states[0].valid);
        assert_eq!(states[0].last_sz, 42);
        assert!((states[0].last_px - 10.05).abs() < 1e-6);
    }

    #[test]
    fn quote_then_trade_keeps_nbbo() {
        let mut states = vec![NbboRow::default()];
        let scale = PRICE_TICK_SCALE as f64;
        merge_quote(&mut states, sample_quote(
            (10.0 * scale) as u64,
            (10.02 * scale) as u64,
        ));
        merge_trade(&mut states, 0, (10.01 * scale) as u64, 7);
        assert!(states[0].valid);
        assert_eq!(states[0].last_sz, 7);
        assert!((states[0].bid - 10.0).abs() < 1e-6);
    }
}

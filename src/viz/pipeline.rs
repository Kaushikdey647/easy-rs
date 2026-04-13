//! Dedicated aggregator thread: pops [`FeedMsg`](crate::cold_path::FeedMsg), downsamples quotes into
//! fixed buckets, runs Lee–Ready on trades, updates heatmap for the focused symbol, publishes
//! [`DashboardSnapshot`] via [`arc_swap::ArcSwap`].

use crate::cold_path::feed_msg::FeedMsg;
use crate::cold_path::ticks::PRICE_TICK_SCALE;
use crate::viz::classify::{lee_ready_signed_volume, price_to_row};
use arc_swap::ArcSwap;
use crossbeam_queue::ArrayQueue;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Downsample grid size (power of two for cheap masking).
pub const NBUCKETS: usize = 1024;
/// Rolling window (seconds) covered by the bucket grid in aggregate.
pub const WINDOW_SEC: f64 = 120.0;
pub const HEATMAP_TIME_COLS: usize = 256;
pub const HEATMAP_PRICE_ROWS: usize = 128;

const DELTA_CAP: usize = 512;
const AGG_BATCH: usize = 256;

#[inline]
pub fn bucket_ns_from_window() -> u64 {
    (((WINDOW_SEC / NBUCKETS as f64) * 1_000_000_000.0).round() as u64).max(1)
}

#[inline]
pub fn bucket_key(ts_ns: u64, bucket_ns: u64) -> u64 {
    ts_ns / bucket_ns
}

#[inline]
pub fn bucket_slot_idx(bucket_key: u64) -> usize {
    bucket_key as usize & (NBUCKETS - 1)
}

#[derive(Clone, Copy, Default)]
struct BucketSlot {
    valid: bool,
    key: u64,
    bid: f64,
    ask: f64,
    lat_ms: f32,
}

struct AggSymbol {
    slots: [BucketSlot; NBUCKETS],
    last_bid_ticks: u64,
    last_ask_ticks: u64,
    have_nbbo: bool,
    last_trade_px: Option<u64>,
    cum_delta: i64,
    delta_t: Vec<f64>,
    delta_cum: Vec<i64>,
    heatmap: Vec<f32>,
    heatmap_seq: u64,
    price_lo: f64,
    price_hi: f64,
}

impl AggSymbol {
    fn new() -> Self {
        Self {
            slots: [BucketSlot::default(); NBUCKETS],
            last_bid_ticks: 0,
            last_ask_ticks: 0,
            have_nbbo: false,
            last_trade_px: None,
            cum_delta: 0,
            delta_t: Vec::with_capacity(64),
            delta_cum: Vec::with_capacity(64),
            heatmap: vec![0.0f32; HEATMAP_TIME_COLS * HEATMAP_PRICE_ROWS],
            heatmap_seq: 0,
            price_lo: f64::MAX,
            price_hi: f64::MIN,
        }
    }

    #[inline]
    fn ticks_to_f64(px_ticks: u64) -> f64 {
        px_ticks as f64 / PRICE_TICK_SCALE as f64
    }

    fn update_heatmap(&mut self, bid_px: f64, ask_px: f64, bid_sz: f64, ask_sz: f64) {
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

/// Pre-built plot series for one symbol (aggregator-owned layout; UI slices by viewport).
#[derive(Clone, Default)]
pub struct SymbolRender {
    pub t_rel: Vec<f64>,
    pub bid: Vec<f64>,
    pub ask: Vec<f64>,
    pub lat_ms: Vec<f32>,
    pub delta_t_rel: Vec<f64>,
    pub delta_cum: Vec<i64>,
    pub cum_delta: i64,
    /// Filled only for the currently focused symbol (same layout as legacy viz).
    pub heatmap: Vec<f32>,
    pub price_lo: f64,
    pub price_hi: f64,
}

#[derive(Clone, Default)]
pub struct DashboardSnapshot {
    pub symbols: Vec<SymbolRender>,
    pub heatmap_version: u64,
}

pub struct DashboardPipeline {
    pub snapshot: Arc<ArcSwap<DashboardSnapshot>>,
    pub heatmap_focus_symbol: Arc<AtomicU16>,
    pub feed_dropped: Arc<AtomicU64>,
}

impl DashboardPipeline {
    pub fn new(symbol_count: usize, feed_dropped: Arc<AtomicU64>) -> Arc<Self> {
        let mut snap = DashboardSnapshot::default();
        snap.symbols = (0..symbol_count).map(|_| SymbolRender::default()).collect();
        Arc::new(Self {
            snapshot: Arc::new(ArcSwap::from_pointee(snap)),
            heatmap_focus_symbol: Arc::new(AtomicU16::new(0)),
            feed_dropped,
        })
    }

    pub fn spawn_aggregator(self: &Arc<Self>, queue: Arc<ArrayQueue<FeedMsg>>, shutdown: Arc<AtomicBool>) {
        let me = Arc::clone(self);
        let q = Arc::clone(&queue);
        let sd = Arc::clone(&shutdown);
        std::thread::spawn(move || aggregator_loop(me, q, sd));
    }
}

fn process_msg(
    states: &mut [AggSymbol],
    msg: FeedMsg,
    focus_sym: u16,
    bucket_ns: u64,
    heatmap_version: &mut u64,
) {
    match msg {
        FeedMsg::Quote(ev) => {
            let id = ev.symbol_id as usize;
            if id >= states.len() {
                return;
            }
            let st = &mut states[id];
            let bid = AggSymbol::ticks_to_f64(ev.bid_px_ticks);
            let ask = AggSymbol::ticks_to_f64(ev.ask_px_ticks);
            st.last_bid_ticks = ev.bid_px_ticks;
            st.last_ask_ticks = ev.ask_px_ticks;
            st.have_nbbo = true;

            let bk = bucket_key(ev.ts_ns, bucket_ns);
            let idx = bucket_slot_idx(bk);
            let slot = &mut st.slots[idx];
            if !slot.valid || slot.key != bk {
                *slot = BucketSlot {
                    valid: true,
                    key: bk,
                    bid,
                    ask,
                    lat_ms: 0.0,
                };
            }
            slot.bid = bid;
            slot.ask = ask;
            let lat_ns = ev.local_rx_ns.saturating_sub(ev.ts_ns);
            slot.lat_ms = (lat_ns as f64 / 1_000_000.0).clamp(-9999.0, 9999.0) as f32;

            if focus_sym == ev.symbol_id {
                st.update_heatmap(bid, ask, ev.bid_sz as f64, ev.ask_sz as f64);
                *heatmap_version = heatmap_version.wrapping_add(1);
            }
        }
        FeedMsg::Trade {
            symbol_id,
            px_ticks,
            sz,
            ts_ns,
        } => {
            let id = symbol_id as usize;
            if id >= states.len() {
                return;
            }
            let st = &mut states[id];
            if !st.have_nbbo {
                return;
            }
            let (signed, last_px) = lee_ready_signed_volume(
                px_ticks,
                sz,
                st.last_bid_ticks,
                st.last_ask_ticks,
                st.last_trade_px,
            );
            if let Some(p) = last_px {
                st.last_trade_px = Some(p);
            }
            st.cum_delta = st.cum_delta.saturating_add(signed);
            if signed != 0 {
                let exch_sec = ts_ns as f64 / 1_000_000_000.0;
                st.delta_t.push(exch_sec);
                st.delta_cum.push(st.cum_delta);
                while st.delta_t.len() > DELTA_CAP {
                    let drop = st.delta_t.len() - DELTA_CAP;
                    st.delta_t.drain(0..drop);
                    st.delta_cum.drain(0..drop);
                }
            }
        }
    }
}

fn build_snapshot(
    states: &[AggSymbol],
    bucket_sec: f64,
    heatmap_version: u64,
    focus_sym: u16,
    scratch: &mut Vec<(u64, f64, f64, f32)>,
) -> DashboardSnapshot {
    let mut symbols = Vec::with_capacity(states.len());
    for (sid, st) in states.iter().enumerate() {
        scratch.clear();
        for i in 0..NBUCKETS {
            let s = &st.slots[i];
            if s.valid {
                scratch.push((s.key, s.bid, s.ask, s.lat_ms));
            }
        }
        scratch.sort_by_key(|p| p.0);

        let anchor = scratch
            .first()
            .map(|(k, _, _, _)| *k as f64 * bucket_sec)
            .unwrap_or(0.0);

        let n = scratch.len();
        let mut t_rel = Vec::with_capacity(n);
        let mut bid = Vec::with_capacity(n);
        let mut ask = Vec::with_capacity(n);
        let mut lat_ms = Vec::with_capacity(n);
        for (k, b, a, l) in scratch.iter().copied() {
            t_rel.push(k as f64 * bucket_sec - anchor);
            bid.push(b);
            ask.push(a);
            lat_ms.push(l);
        }

        let mut delta_t_rel = Vec::with_capacity(st.delta_t.len());
        for &t in &st.delta_t {
            delta_t_rel.push(t - anchor);
        }

        let heatmap = if sid == focus_sym as usize {
            st.heatmap.clone()
        } else {
            Vec::new()
        };

        symbols.push(SymbolRender {
            t_rel,
            bid,
            ask,
            lat_ms,
            delta_t_rel,
            delta_cum: st.delta_cum.clone(),
            cum_delta: st.cum_delta,
            heatmap,
            price_lo: st.price_lo,
            price_hi: st.price_hi,
        });
    }
    DashboardSnapshot {
        symbols,
        heatmap_version,
    }
}

fn aggregator_loop(pipe: Arc<DashboardPipeline>, queue: Arc<ArrayQueue<FeedMsg>>, shutdown: Arc<AtomicBool>) {
    let n_sym = pipe.snapshot.load().symbols.len();
    let mut states: Vec<AggSymbol> = (0..n_sym).map(|_| AggSymbol::new()).collect();
    let bucket_ns = bucket_ns_from_window();
    let bucket_sec = WINDOW_SEC / NBUCKETS as f64;
    let mut heatmap_version = 0u64;
    let mut scratch: Vec<(u64, f64, f64, f32)> = Vec::with_capacity(NBUCKETS);

    while !shutdown.load(Ordering::Relaxed) {
        let mut did = 0usize;
        while did < AGG_BATCH {
            match queue.pop() {
                Some(msg) => {
                    let focus = pipe.heatmap_focus_symbol.load(Ordering::Relaxed);
                    process_msg(&mut states, msg, focus, bucket_ns, &mut heatmap_version);
                    did += 1;
                }
                None => break,
            }
        }

        if did == 0 {
            std::thread::sleep(Duration::from_micros(50));
            continue;
        }

        let focus = pipe.heatmap_focus_symbol.load(Ordering::Relaxed);
        let snap = build_snapshot(
            &states,
            bucket_sec,
            heatmap_version,
            focus,
            &mut scratch,
        );
        pipe.snapshot.store(Arc::new(snap));
    }

    let focus = pipe.heatmap_focus_symbol.load(Ordering::Relaxed);
    let snap = build_snapshot(
        &states,
        bucket_sec,
        heatmap_version,
        focus,
        &mut scratch,
    );
    pipe.snapshot.store(Arc::new(snap));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_slot_mask_is_power_of_two() {
        assert_eq!(NBUCKETS.count_ones(), 1);
    }

    #[test]
    fn bucket_key_monotonic() {
        let bn = bucket_ns_from_window();
        let a = bucket_key(1000, bn);
        let b = bucket_key(1000 + bn, bn);
        assert_eq!(b, a + 1);
    }

    #[test]
    fn slot_index_stable_for_same_key() {
        let k = 424242u64;
        assert_eq!(bucket_slot_idx(k), bucket_slot_idx(k));
    }

    #[test]
    fn quote_then_trade_updates_delta() {
        let mut states = vec![AggSymbol::new()];
        let bn = bucket_ns_from_window();
        let mut hv = 0u64;
        let exch = 1_000_000_000u64;
        let local = exch + 1_500_000;
        process_msg(
            &mut states,
            FeedMsg::Quote(crate::hot_path::quote_event::QuoteEvent {
                symbol_id: 0,
                bid_px_ticks: 100,
                ask_px_ticks: 200,
                bid_sz: 1,
                ask_sz: 1,
                ts_ns: exch,
                local_rx_ns: local,
                ingest_mono_ns: 0,
            }),
            0,
            bn,
            &mut hv,
        );
        process_msg(
            &mut states,
            FeedMsg::Trade {
                symbol_id: 0,
                px_ticks: 180,
                sz: 5,
                ts_ns: exch + 200_000_000,
            },
            0,
            bn,
            &mut hv,
        );
        let mut scratch = Vec::new();
        let snap = build_snapshot(&states, WINDOW_SEC / NBUCKETS as f64, hv, 0, &mut scratch);
        assert!(!snap.symbols[0].delta_t_rel.is_empty());
    }
}

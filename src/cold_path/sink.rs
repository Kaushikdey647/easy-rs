//! Where normalized quotes go after the WebSocket decode (ring, last-NBBO mirror, TUI/aggregator queue).

use crate::cold_path::feed_msg::FeedMsg;
use crate::hot_path::quote_event::QuoteEvent;
use crate::hot_path::ring::{QuoteRing, RingPushOutcome, try_push_drop_newest};
use crossbeam_queue::ArrayQueue;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Optional outputs for each quote. All fields are independent: enable only what you need.
#[derive(Clone, Default)]
pub struct AlpacaQuoteSink {
    /// Bounded queue + drop counter (legacy path); `dropped` must be set when this is set.
    pub ring: Option<QuoteRing>,
    pub dropped: Option<Arc<AtomicU64>>,
    /// Latest quote per `symbol_id` for headless logging or metrics.
    pub last_nbbo: Option<Arc<Mutex<Vec<Option<QuoteEvent>>>>>,
    /// Aggregator / TUI handoff: lock-free queue; **drop-newest** when full (keeps backlog age bounded).
    pub feed_queue: Option<Arc<ArrayQueue<FeedMsg>>>,
    pub feed_dropped: Option<Arc<AtomicU64>>,
}

impl AlpacaQuoteSink {
    /// Headless stream: only track last NBBO per symbol (for shutdown logs).
    pub fn headless_last_nbbo(symbol_count: usize) -> (Self, Arc<Mutex<Vec<Option<QuoteEvent>>>>) {
        let last = Arc::new(Mutex::new(vec![None; symbol_count]));
        let sink = Self {
            last_nbbo: Some(Arc::clone(&last)),
            ..Default::default()
        };
        (sink, last)
    }

    /// Terminal UI / aggregator: push quotes/trades to `feed_queue` only (no mutex on the Tokio thread).
    pub fn dashboard(feed: Arc<ArrayQueue<FeedMsg>>, feed_dropped: Arc<AtomicU64>) -> Self {
        Self {
            feed_queue: Some(feed),
            feed_dropped: Some(feed_dropped),
            ..Default::default()
        }
    }

    /// Ring path (drops newest when full); rarely needed now that the order book is removed.
    pub fn with_ring(ring: QuoteRing, dropped: Arc<AtomicU64>) -> Self {
        Self {
            ring: Some(ring),
            dropped: Some(dropped),
            ..Default::default()
        }
    }

    #[inline]
    fn try_push_feed(&self, msg: FeedMsg) {
        let Some(q) = &self.feed_queue else {
            return;
        };
        if q.push(msg).is_err() {
            if let Some(d) = &self.feed_dropped {
                d.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[inline]
    pub fn on_quote(&self, ev: QuoteEvent) {
        if let Some(ring) = &self.ring {
            if let Some(dropped) = &self.dropped {
                if try_push_drop_newest(ring.as_ref(), ev) == RingPushOutcome::DroppedNewest {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        if let Some(buf) = &self.last_nbbo {
            let mut g = buf.lock();
            let i = ev.symbol_id as usize;
            if i < g.len() {
                g[i] = Some(ev);
            }
        }
        self.try_push_feed(FeedMsg::Quote(ev));
    }

    #[inline]
    pub fn on_trade_msg(&self, symbol_id: u16, px_ticks: u64, sz: u64, ts_ns: u64) {
        self.try_push_feed(FeedMsg::Trade {
            symbol_id,
            px_ticks,
            sz,
            ts_ns,
        });
    }
}

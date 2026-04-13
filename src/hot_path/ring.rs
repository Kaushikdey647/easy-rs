//! Bounded lock-free handoff using `crossbeam_queue::ArrayQueue`.

use super::quote_event::QuoteEvent;
use crossbeam_queue::ArrayQueue;
use std::sync::Arc;

/// Bounded queue between ColdPath (producer) and HotPath (consumer).
pub type QuoteRing = Arc<ArrayQueue<QuoteEvent>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RingPushOutcome {
    Enqueued,
    DroppedNewest,
}

pub fn new_quote_ring(capacity: usize) -> QuoteRing {
    Arc::new(ArrayQueue::new(capacity))
}

/// Try to push; if full, **drop the new event** (keep queue latency bounded).
#[inline]
pub fn try_push_drop_newest(ring: &ArrayQueue<QuoteEvent>, event: QuoteEvent) -> RingPushOutcome {
    if ring.push(event).is_ok() {
        RingPushOutcome::Enqueued
    } else {
        RingPushOutcome::DroppedNewest
    }
}

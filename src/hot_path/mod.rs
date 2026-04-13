//! HotPath: single consumer thread, bounded queue, NBBO apply into `orderbook-rs`.
//!
//! No per-tick logging here; drop policy is explicit when the queue is full.

pub mod applier;
pub mod quote_event;
pub mod ring;

pub use applier::run_consumer;
pub use quote_event::QuoteEvent;
pub use ring::{new_quote_ring, QuoteRing, RingPushOutcome};

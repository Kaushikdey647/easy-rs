//! Quote normalization types and optional bounded handoff ([`QuoteRing`]).

pub mod quote_event;
pub mod ring;

pub use quote_event::QuoteEvent;
pub use ring::{new_quote_ring, QuoteRing, RingPushOutcome};

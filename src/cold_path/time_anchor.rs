//! Monotonic anchor for [`crate::hot_path::quote_event::QuoteEvent::ingest_mono_ns`].

use std::sync::OnceLock;
use std::time::Instant;

static PROGRAM_START: OnceLock<Instant> = OnceLock::new();

/// Pin the program-wide start instant (call from `main` before the feed for consistent timing).
#[inline]
pub fn ensure_started() {
    let _ = PROGRAM_START.get_or_init(Instant::now);
}

#[inline]
pub fn program_start() -> Instant {
    *PROGRAM_START.get_or_init(Instant::now)
}

/// Nanoseconds since [`program_start`] from `now` (typically `Instant::now()` right after decode).
#[inline]
pub fn mono_ns_since_start(now: Instant) -> u64 {
    now.duration_since(program_start()).as_nanos() as u64
}

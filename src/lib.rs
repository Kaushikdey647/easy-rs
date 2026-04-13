//! easy-rs: Alpaca market data (via [`apca`]) into [`orderbook_rs`] with a hot/cold split.
//!
//! ## Latency model (SLO-oriented)
//!
//! **End-to-end freshness** (exchange clock → local book):
//! `T_e2e ≈ T_network + T_tls + T_ws_frame + T_json + T_dispatch + T_ring + T_book`
//!
//! - **`T_book`**: time from a [`hot_path::QuoteEvent`] being visible to the consumer thread
//!   until [`orderbook_rs::DefaultOrderBook`] reflects the NBBO. Target this for microstructure
//!   research on the **local** machine; it excludes Alpaca propagation delay.
//! - **`T_ui`**: separate coalesced path for browsers (milliseconds is acceptable).
//!
//! **HotPath** (tight): bounded handoff ([`hot_path::QuoteRing`]) → single consumer applies
//! NBBO into `orderbook-rs` without logging.
//!
//! **ColdPath** (Tokio): WebSocket I/O, JSON decode, tick normalization, subscribe/resubscribe,
//! tracing, and shutdown.

pub mod cold_path;
pub mod hot_path;
pub mod orderbook_store;
pub mod viz;

pub use cold_path::symbols::SymbolRegistry;
pub use cold_path::ticks;
pub use cold_path::{AlpacaFeedConfig, AlpacaFeedSource};
pub use hot_path::quote_event::QuoteEvent;
pub use hot_path::{new_quote_ring, run_consumer};
pub use orderbook_store::BookRegistry;

/// Install `tracing` with `RUST_LOG` (default `info`). Set `EASY_RS_TRACING_SPANS=1` to log span close
/// timings on stderr (use `RUST_LOG=trace` for verbose spans). With `--features tracy`, also set
/// `TRACY=1` to send frames to Tracy (if enabled in your subscriber setup).
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    #[cfg(feature = "tracy")]
    {
        if std::env::var("TRACY").as_deref() == Ok("1") {
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;
            use tracing_subscriber::Layer;
            let fmt = tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(filter.clone());
            tracing_subscriber::registry()
                .with(tracing_tracy::TracyLayer::default())
                .with(fmt)
                .init();
            return;
        }
    }

    let mut fmt = tracing_subscriber::fmt().with_env_filter(filter);
    if std::env::var("EASY_RS_TRACING_SPANS").as_deref() == Ok("1") {
        fmt = fmt.with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE);
    }
    fmt.init();
}

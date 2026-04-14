//! easy-rs: Alpaca market data (via [`apca`]) on a **cold path** (Tokio WebSocket + JSON),
//! with optional **viz** and an optional bounded quote ring.
//!
//! Quotes and trades come straight from Alpaca; NBBO is not mirrored into a local order book—
//! the stream already carries top-of-book quotes.
//!
//! **ColdPath**: WebSocket I/O, JSON decode, tick normalization, [`AlpacaQuoteSink`] fan-out,
//! tracing, shutdown.
//!
//! ## Indicators and execution (optional features)
//!
//! - `indicators`: TA features ( [`ta`] ) analogous to shunya `finta` columns.
//! - `barter`: backtest / mock execution via [Barter](https://docs.rs/barter/0.12.4/barter/); live
//!   Alpaca order placement remains [`apca`] until a dedicated execution adapter exists.
//!
//! **Future:** a `quant` feature may add [RustQuant](https://crates.io/crates/RustQuant) for
//! pricing and statistics—kept off the hot streaming path.

pub mod cold_path;
pub mod data;
pub mod hot_path;
pub mod viz;

#[cfg(feature = "indicators")]
pub mod indicators;

pub use cold_path::symbols::SymbolRegistry;
pub use cold_path::ticks;
pub use cold_path::{AlpacaFeedConfig, AlpacaFeedSource, AlpacaQuoteSink};
pub use hot_path::quote_event::QuoteEvent;
pub use hot_path::{QuoteRing, new_quote_ring};

/// Install `tracing` with `RUST_LOG` (default `info`). Set `EASY_RS_TRACING_SPANS=1` to log span close
/// timings on stderr (use `RUST_LOG=trace` for verbose spans). With `--features tracy`, also set
/// `TRACY=1` to send frames to Tracy (if enabled in your subscriber setup).
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    #[cfg(feature = "tracy")]
    {
        if std::env::var("TRACY").as_deref() == Ok("1") {
            use tracing_subscriber::Layer;
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;
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

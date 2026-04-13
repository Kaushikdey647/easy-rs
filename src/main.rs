//! Alpaca quote stream (ColdPath) → bounded ring → single consumer (HotPath) → `orderbook-rs`.

use apca::Error as AlpacaError;
use easy_rs::cold_path::{run_alpaca_quotes, AlpacaFeedConfig, AlpacaFeedSource};
use easy_rs::hot_path::ring::new_quote_ring;
use easy_rs::hot_path::run_consumer;
use easy_rs::{BookRegistry, SymbolRegistry};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tracing::info;

fn parse_feed_source() -> AlpacaFeedSource {
    match std::env::var("EASY_RS_ALPACA_FEED")
        .unwrap_or_else(|_| "iex".into())
        .to_lowercase()
        .as_str()
    {
        "sip" => AlpacaFeedSource::Sip,
        _ => AlpacaFeedSource::Iex,
    }
}

fn symbols_from_env() -> Vec<String> {
    std::env::var("EASY_RS_SYMBOLS")
        .unwrap_or_else(|_| "SPY".into())
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn ring_capacity() -> usize {
    std::env::var("EASY_RS_RING_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4096)
}

/// Core logic uses `apca::Error` end-to-end so `?` works without boxing to `dyn Error`.
async fn run() -> Result<(), AlpacaError> {
    // Load `.env` from the current working directory if present. Does not override existing vars.
    let _ = dotenvy::dotenv();

    easy_rs::init_tracing();

    let symbols = symbols_from_env();
    if symbols.is_empty() {
        return Err(AlpacaError::Str(
            "EASY_RS_SYMBOLS must list at least one symbol".into(),
        ));
    }

    let registry = SymbolRegistry::new(symbols);
    let book_registry = BookRegistry::from_sorted_symbols(registry.symbols_sorted());
    let consumer_books: Vec<_> = book_registry.books().to_vec();

    let ring = new_quote_ring(ring_capacity());
    let shutdown = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicU64::new(0));

    let hot_ring = Arc::clone(&ring);
    let hot_shutdown = Arc::clone(&shutdown);
    let hot_handle = std::thread::spawn(move || {
        run_consumer(hot_ring, consumer_books, hot_shutdown);
    });

    let shutdown_signal = Arc::clone(&shutdown);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown_signal.store(true, Ordering::SeqCst);
    });

    let cfg = AlpacaFeedConfig {
        source: parse_feed_source(),
        symbols: registry.symbols_sorted().to_vec(),
    };

    run_alpaca_quotes(
        cfg,
        registry,
        ring,
        Arc::clone(&dropped),
        Arc::clone(&shutdown),
        None,
    )
    .await?;

    shutdown.store(true, Ordering::SeqCst);
    hot_handle.join().expect("hot thread join");

    info!(
        ring_dropped_events = dropped.load(Ordering::Relaxed),
        "feed stopped; printing last NBBO snapshot (best bid/ask ticks)"
    );

    for (i, book) in book_registry.books().iter().enumerate() {
        info!(
            symbol_id = i,
            best_bid = ?book.best_bid(),
            best_ask = ?book.best_ask(),
            "book top"
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        tracing::error!(?e, "easy-rs exited with error");
        std::process::exit(1);
    }
}

//! Alpaca quote/trade stream (cold path) with optional sinks—no local order book.

use apca::Error as AlpacaError;
use easy_rs::SymbolRegistry;
use easy_rs::cold_path::{AlpacaFeedConfig, AlpacaFeedSource, AlpacaQuoteSink, run_alpaca_quotes};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Core logic uses `apca::Error` end-to-end so `?` works without boxing to `dyn Error`.
async fn run() -> Result<(), AlpacaError> {
    let _ = dotenvy::dotenv();

    easy_rs::init_tracing();
    easy_rs::cold_path::time_anchor::ensure_started();

    let symbols = symbols_from_env();
    if symbols.is_empty() {
        return Err(AlpacaError::Str(
            "EASY_RS_SYMBOLS must list at least one symbol".into(),
        ));
    }

    let registry = SymbolRegistry::new(symbols);
    let (sink, last_nbbo) = AlpacaQuoteSink::headless_last_nbbo(registry.len());

    let shutdown = Arc::new(AtomicBool::new(false));

    let shutdown_signal = Arc::clone(&shutdown);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown_signal.store(true, Ordering::SeqCst);
    });

    let cfg = AlpacaFeedConfig {
        source: parse_feed_source(),
        symbols: registry.symbols_sorted().to_vec(),
    };

    run_alpaca_quotes(cfg, registry, sink, Arc::clone(&shutdown)).await?;

    shutdown.store(true, Ordering::SeqCst);

    info!("feed stopped; last NBBO snapshot (integer ticks, 1e-8 dollars per tick unit)");

    let snap = last_nbbo.lock();
    for (i, q) in snap.iter().enumerate() {
        if let Some(ev) = q {
            info!(
                symbol_id = i,
                bid_px_ticks = ev.bid_px_ticks,
                ask_px_ticks = ev.ask_px_ticks,
                bid_sz = ev.bid_sz,
                ask_sz = ev.ask_sz,
                "nbbo"
            );
        }
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

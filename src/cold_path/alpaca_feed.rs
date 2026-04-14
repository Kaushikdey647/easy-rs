//! Alpaca v2 stock websocket (quotes + trades) → [`AlpacaQuoteSink`] (viz / last NBBO / optional ring).

use crate::cold_path::sink::AlpacaQuoteSink;
use crate::cold_path::symbols::SymbolRegistry;
use crate::cold_path::ticks::{num_to_tick_u64, num_to_whole_shares};
use crate::data::OhlcvBar;
use crate::hot_path::quote_event::QuoteEvent;
use apca::data::v2::stream::{Data, IEX, MarketData, RealtimeData, SIP, Source, drive};
use apca::{ApiInfo, Client, Error};
use chrono::{DateTime, Utc};
use futures::FutureExt;
use futures::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tracing::instrument;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlpacaFeedSource {
    Iex,
    Sip,
}

#[derive(Clone, Debug)]
pub struct AlpacaFeedConfig {
    pub source: AlpacaFeedSource,
    pub symbols: Vec<String>,
    /// When `true`, subscribe to Alpaca aggregate bars for `symbols` and forward them via [`AlpacaQuoteSink::on_bar`].
    pub subscribe_bars: bool,
}

pub async fn run_alpaca_quotes(
    cfg: AlpacaFeedConfig,
    registry: SymbolRegistry,
    sink: AlpacaQuoteSink,
    shutdown: Arc<AtomicBool>,
) -> Result<(), Error> {
    match cfg.source {
        AlpacaFeedSource::Iex => run_inner::<IEX>(cfg, registry, sink, shutdown).await,
        AlpacaFeedSource::Sip => run_inner::<SIP>(cfg, registry, sink, shutdown).await,
    }
}

async fn run_inner<S>(
    cfg: AlpacaFeedConfig,
    registry: SymbolRegistry,
    sink: AlpacaQuoteSink,
    shutdown: Arc<AtomicBool>,
) -> Result<(), Error>
where
    S: Source,
{
    crate::cold_path::time_anchor::ensure_started();
    let api_info = ApiInfo::from_env()?;
    let client = Client::new(api_info);
    let (mut stream, mut subscription) = client.subscribe::<RealtimeData<S>>().await?;

    let mut data = MarketData::default();
    data.set_quotes(cfg.symbols.clone());
    data.set_trades(cfg.symbols.clone());
    if cfg.subscribe_bars {
        data.set_bars(cfg.symbols.clone());
    }

    let sub = subscription.subscribe(&data).boxed();
    let drove = drive(sub, &mut stream).await;
    let subscribe_result = match drove {
        Ok(x) => x,
        Err(e) => {
            tracing::error!(?e, "subscribe drive returned user/control frame");
            return Err(Error::Str(
                "subscription handshake interrupted by stream traffic".into(),
            ));
        }
    };
    match subscribe_result {
        Ok(Ok(())) => {}
        Ok(Err(alpaca_err)) => return Err(alpaca_err),
        Err(sub_err) => {
            return Err(Error::Str(format!("subscription sink: {sub_err:?}").into()));
        }
    }

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let next = stream.next().await;
        let Some(frame) = next else {
            break;
        };

        match frame {
            Ok(Ok(payload)) => {
                if let Err(e) = handle_payload(&registry, &sink, payload) {
                    tracing::warn!(?e, "payload handling error");
                }
            }
            Ok(Err(json_err)) => tracing::warn!(%json_err, "stream json error"),
            Err(ws_err) => {
                tracing::error!(?ws_err, "websocket error");
                return Err(Error::WebSocket(ws_err));
            }
        }
    }

    Ok(())
}

#[inline]
fn wall_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn handle_payload(
    registry: &SymbolRegistry,
    sink: &AlpacaQuoteSink,
    payload: Data,
) -> Result<(), &'static str> {
    match payload {
        Data::Quote(q) => handle_quote(registry, sink, q),
        Data::Trade(t) => {
            handle_trade(registry, sink, t)?;
            Ok(())
        }
        Data::Bar(b) => handle_bar(registry, sink, b),
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

#[instrument(skip_all, fields(symbol = %q.symbol))]
fn handle_quote(
    registry: &SymbolRegistry,
    sink: &AlpacaQuoteSink,
    q: apca::data::v2::stream::Quote,
) -> Result<(), &'static str> {
    let ingest_instant = Instant::now();
    let ingest_mono_ns = crate::cold_path::time_anchor::mono_ns_since_start(ingest_instant);
    let Some(symbol_id) = registry.id(q.symbol.as_str()) else {
        return Ok(());
    };
    let Some(bid_px) = num_to_tick_u64(&q.bid_price) else {
        return Err("bid price parse");
    };
    let Some(ask_px) = num_to_tick_u64(&q.ask_price) else {
        return Err("ask price parse");
    };
    let bid_sz = num_to_whole_shares(&q.bid_size);
    let ask_sz = num_to_whole_shares(&q.ask_size);
    let ts_ns = quote_ts_ns(&q.timestamp);
    let local_rx_ns = wall_ns();

    let ev = QuoteEvent {
        symbol_id,
        bid_px_ticks: bid_px,
        ask_px_ticks: ask_px,
        bid_sz,
        ask_sz,
        ts_ns,
        local_rx_ns,
        ingest_mono_ns,
    };

    sink.on_quote(ev);
    Ok(())
}

#[instrument(skip_all, fields(symbol = %b.symbol))]
fn handle_bar(
    registry: &SymbolRegistry,
    sink: &AlpacaQuoteSink,
    b: apca::data::v2::stream::Bar,
) -> Result<(), &'static str> {
    let Some(symbol_id) = registry.id(b.symbol.as_str()) else {
        return Ok(());
    };
    sink.on_bar(symbol_id, OhlcvBar::from(&b));
    Ok(())
}

#[instrument(skip_all, fields(symbol = %t.symbol))]
fn handle_trade(
    registry: &SymbolRegistry,
    sink: &AlpacaQuoteSink,
    t: apca::data::v2::stream::Trade,
) -> Result<(), &'static str> {
    let Some(symbol_id) = registry.id(t.symbol.as_str()) else {
        return Ok(());
    };
    let Some(px) = num_to_tick_u64(&t.trade_price) else {
        return Err("trade price parse");
    };
    let sz = num_to_whole_shares(&t.trade_size);
    let ts_ns = quote_ts_ns(&t.timestamp);

    sink.on_trade_msg(symbol_id, px, sz, ts_ns);
    Ok(())
}

fn quote_ts_ns(ts: &DateTime<Utc>) -> u64 {
    let secs = ts.timestamp().max(0) as u64;
    let sub = ts.timestamp_subsec_nanos() as u64;
    secs.saturating_mul(1_000_000_000).saturating_add(sub)
}

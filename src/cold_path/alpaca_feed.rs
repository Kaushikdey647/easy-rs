//! Alpaca v2 stock websocket (quotes + trades) → [`crate::hot_path::QuoteRing`] + optional [`crate::viz::VizStore`].

use crate::cold_path::symbols::SymbolRegistry;
use crate::cold_path::ticks::{num_to_tick_u64, num_to_whole_shares};
use crate::hot_path::quote_event::QuoteEvent;
use crate::hot_path::ring::{try_push_drop_newest, QuoteRing, RingPushOutcome};
use crate::viz::VizStore;
use apca::data::v2::stream::{drive, Data, IEX, MarketData, RealtimeData, SIP, Source};
use apca::{ApiInfo, Client, Error};
use chrono::{DateTime, Utc};
use crossbeam_queue::ArrayQueue;
use futures::FutureExt;
use futures::StreamExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
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
}

pub async fn run_alpaca_quotes(
    cfg: AlpacaFeedConfig,
    registry: SymbolRegistry,
    ring: QuoteRing,
    dropped: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    viz: Option<Arc<VizStore>>,
) -> Result<(), Error> {
    match cfg.source {
        AlpacaFeedSource::Iex => run_inner::<IEX>(cfg, registry, ring, dropped, shutdown, viz).await,
        AlpacaFeedSource::Sip => run_inner::<SIP>(cfg, registry, ring, dropped, shutdown, viz).await,
    }
}

async fn run_inner<S>(
    cfg: AlpacaFeedConfig,
    registry: SymbolRegistry,
    ring: QuoteRing,
    dropped: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    viz: Option<Arc<VizStore>>,
) -> Result<(), Error>
where
    S: Source,
{
    let api_info = ApiInfo::from_env()?;
    let client = Client::new(api_info);
    let (mut stream, mut subscription) = client.subscribe::<RealtimeData<S>>().await?;

    let mut data = MarketData::default();
    data.set_quotes(cfg.symbols.clone());
    data.set_trades(cfg.symbols.clone());

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
        Err(sink_err) => {
            return Err(Error::Str(format!("subscription sink: {sink_err:?}").into()));
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
                if let Err(e) = handle_payload(&registry, ring.as_ref(), &dropped, viz.as_ref(), payload) {
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
    ring: &ArrayQueue<QuoteEvent>,
    dropped: &AtomicU64,
    viz: Option<&Arc<VizStore>>,
    payload: Data,
) -> Result<(), &'static str> {
    match payload {
        Data::Quote(q) => handle_quote(registry, ring, dropped, viz, q),
        Data::Trade(t) => {
            handle_trade(registry, viz, t)?;
            Ok(())
        }
        Data::Bar(_) => Ok(()),
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

#[instrument(skip_all, fields(symbol = %q.symbol))]
fn handle_quote(
    registry: &SymbolRegistry,
    ring: &ArrayQueue<QuoteEvent>,
    dropped: &AtomicU64,
    viz: Option<&Arc<VizStore>>,
    q: apca::data::v2::stream::Quote,
) -> Result<(), &'static str> {
    let local_rx_ns = wall_ns();
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

    let ev = QuoteEvent {
        symbol_id,
        bid_px_ticks: bid_px,
        ask_px_ticks: ask_px,
        bid_sz,
        ask_sz,
        ts_ns,
        local_rx_ns,
    };

    if try_push_drop_newest(ring, ev) == RingPushOutcome::DroppedNewest {
        dropped.fetch_add(1, Ordering::Relaxed);
    }

    if let Some(v) = viz {
        v.on_quote(symbol_id, bid_px, ask_px, bid_sz, ask_sz, ts_ns, local_rx_ns);
    }
    Ok(())
}

#[instrument(skip_all, fields(symbol = %t.symbol))]
fn handle_trade(
    registry: &SymbolRegistry,
    viz: Option<&Arc<VizStore>>,
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

    if let Some(v) = viz {
        v.on_trade(symbol_id, px, sz, ts_ns);
    }
    Ok(())
}

fn quote_ts_ns(ts: &DateTime<Utc>) -> u64 {
    let secs = ts.timestamp().max(0) as u64;
    let sub = ts.timestamp_subsec_nanos() as u64;
    secs.saturating_mul(1_000_000_000).saturating_add(sub)
}

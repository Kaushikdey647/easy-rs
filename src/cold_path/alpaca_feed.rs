//! Alpaca v2 stock websocket (quotes) → [`crate::hot_path::QuoteRing`].

use crate::cold_path::symbols::SymbolRegistry;
use crate::cold_path::ticks::{num_to_tick_u64, num_to_whole_shares};
use crate::hot_path::quote_event::QuoteEvent;
use crate::hot_path::ring::{try_push_drop_newest, QuoteRing, RingPushOutcome};
use apca::data::v2::stream::{drive, Data, IEX, MarketData, RealtimeData, SIP, Source};
use apca::{ApiInfo, Client, Error};
use chrono::{DateTime, Utc};
use crossbeam_queue::ArrayQueue;
use futures::FutureExt;
use futures::StreamExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

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
) -> Result<(), Error> {
    match cfg.source {
        AlpacaFeedSource::Iex => run_inner::<IEX>(cfg, registry, ring, dropped, shutdown).await,
        AlpacaFeedSource::Sip => run_inner::<SIP>(cfg, registry, ring, dropped, shutdown).await,
    }
}

async fn run_inner<S>(
    cfg: AlpacaFeedConfig,
    registry: SymbolRegistry,
    ring: QuoteRing,
    dropped: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
) -> Result<(), Error>
where
    S: Source,
{
    let api_info = ApiInfo::from_env()?;
    let client = Client::new(api_info);
    let (mut stream, mut subscription) = client.subscribe::<RealtimeData<S>>().await?;

    let mut data = MarketData::default();
    data.set_quotes(cfg.symbols.clone());

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
                if let Err(e) = handle_payload(&registry, ring.as_ref(), &dropped, payload) {
                    tracing::warn!(?e, "quote handling error");
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

fn handle_payload(
    registry: &SymbolRegistry,
    ring: &ArrayQueue<QuoteEvent>,
    dropped: &AtomicU64,
    payload: Data,
) -> Result<(), &'static str> {
    match payload {
        Data::Quote(q) => {
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
            };

            if try_push_drop_newest(ring, ev) == RingPushOutcome::DroppedNewest {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }
        Data::Bar(_) | Data::Trade(_) => Ok(()),
        #[allow(unreachable_patterns)]
        _ => Ok(()),
    }
}

fn quote_ts_ns(ts: &DateTime<Utc>) -> u64 {
    let secs = ts.timestamp().max(0) as u64;
    let sub = ts.timestamp_subsec_nanos() as u64;
    secs.saturating_mul(1_000_000_000).saturating_add(sub)
}

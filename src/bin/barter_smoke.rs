//! One Barter backtest over bundled historic L1/trade events (same JSONL shape as upstream Barter examples).
//!
//! Run: `cargo run --features barter --bin easy-rs-barter-smoke`
//!
//! Live Alpaca order flow remains on [`apca`]; this binary only validates the Barter engine + mock execution path.

use std::sync::Arc;

use barter::backtest::market_data::{BacktestMarketData, MarketDataInMemory};
use barter::backtest::{BacktestArgsConstant, BacktestArgsDynamic, backtest};
use barter::engine::state::{
    EngineState, builder::EngineStateBuilder, global::DefaultGlobalData,
    instrument::data::DefaultInstrumentMarketData, trading::TradingState,
};
use barter::risk::DefaultRiskManager;
use barter::statistic::time::Daily;
use barter::strategy::DefaultStrategy;
use barter::system::config::SystemConfig;
use barter_data::event::DataKind;
use barter_data::streams::consumer::MarketStreamEvent;
use barter_instrument::index::IndexedInstruments;
use barter_instrument::instrument::InstrumentIndex;
use rust_decimal::Decimal;
use serde::Deserialize;
use smol_str::SmolStr;

const CONFIG: &str = include_str!("../../examples_data/barter_backtest_config.json");
const MARKET: &str = include_str!("../../examples_data/barter_market_sample.jsonl");

#[derive(Deserialize)]
struct FileConfig {
    risk_free_return: Decimal,
    system: SystemConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    easy_rs::init_tracing();

    let FileConfig {
        risk_free_return,
        system: SystemConfig {
            instruments,
            executions,
        },
    } = serde_json::from_str(CONFIG)?;

    let instruments = IndexedInstruments::new(instruments);

    let events: Vec<MarketStreamEvent<InstrumentIndex, DataKind>> = MARKET
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| serde_json::from_str(line))
        .collect::<Result<_, _>>()?;

    let market_data = MarketDataInMemory::new(Arc::new(events));
    let time_engine_start = market_data.time_first_event().await?;

    let engine_state = EngineStateBuilder::new(
        &instruments,
        DefaultGlobalData::default(),
        |_| DefaultInstrumentMarketData::default(),
    )
    .time_engine_start(time_engine_start)
    .trading_state(TradingState::Enabled)
    .build();

    let args_constant = Arc::new(BacktestArgsConstant {
        instruments,
        executions,
        market_data,
        summary_interval: Daily,
        engine_state,
    });

    let args_dynamic = BacktestArgsDynamic {
        id: SmolStr::from("easy-rs-smoke"),
        risk_free_return,
        strategy: DefaultStrategy::<EngineState<DefaultGlobalData, DefaultInstrumentMarketData>>::default(),
        risk: DefaultRiskManager::<EngineState<DefaultGlobalData, DefaultInstrumentMarketData>>::default(),
    };

    let summary = backtest(args_constant, args_dynamic).await?;
    summary.trading_summary.print_summary();
    Ok(())
}

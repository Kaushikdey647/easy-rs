//! Technical features aligned with shunya `finta` usage (SMA, RSI, MACD, Bollinger, ATR).
//!
//! Uses the [`ta`] crate (`Next` streaming API). Warm-up semantics differ slightly from pandas
//! rolling windows; compare end-of-series values for validation vs Python.

use crate::data::OhlcvBar;
use ta::DataItem;
use ta::Next;
use ta::indicators::{
    AverageTrueRange, BollingerBands, MovingAverageConvergenceDivergence, RelativeStrengthIndex,
    SimpleMovingAverage,
};

/// Engineered columns matching [`shunya`](https://github.com/Kaushikdey647/shunya) `finTs::_add_features_full` core set.
#[derive(Clone, Debug, PartialEq)]
pub struct ShunyaFeatureRow {
    pub vwap: f64,
    pub sma_50: f64,
    pub sma_200: f64,
    pub rsi_14: f64,
    pub macd: f64,
    pub macd_signal: f64,
    pub bb_upper: f64,
    pub bb_lower: f64,
    pub atr_14: f64,
    pub log_ret: f64,
    pub dist_sma50: f64,
    pub dist_sma200: f64,
    pub bb_width: f64,
    pub bb_position: f64,
    pub macd_hist: f64,
    pub atr_norm: f64,
    pub vol_change: f64,
}

/// Incremental indicator state; call [`Self::push`] for each bar in order.
#[derive(Debug)]
pub struct ShunyaFeatureEngine {
    sma_50: SimpleMovingAverage,
    sma_200: SimpleMovingAverage,
    rsi_14: RelativeStrengthIndex,
    macd: MovingAverageConvergenceDivergence,
    bb: BollingerBands,
    atr_14: AverageTrueRange,
    prev_close: Option<f64>,
    prev_vol: Option<f64>,
}

impl Default for ShunyaFeatureEngine {
    fn default() -> Self {
        Self::new().expect("valid indicator params")
    }
}

impl ShunyaFeatureEngine {
    pub fn new() -> Result<Self, ta::errors::TaError> {
        Ok(Self {
            sma_50: SimpleMovingAverage::new(50)?,
            sma_200: SimpleMovingAverage::new(200)?,
            rsi_14: RelativeStrengthIndex::new(14)?,
            macd: MovingAverageConvergenceDivergence::new(12, 26, 9)?,
            bb: BollingerBands::new(20, 2.0)?,
            atr_14: AverageTrueRange::new(14)?,
            prev_close: None,
            prev_vol: None,
        })
    }

    /// One bar; returns NaN-filled row until indicators stabilize (same as ta `Next` behavior).
    pub fn push(&mut self, bar: &OhlcvBar) -> ShunyaFeatureRow {
        let item = DataItem::builder()
            .open(bar.open)
            .high(bar.high)
            .low(bar.low)
            .close(bar.close)
            .volume(bar.volume)
            .build()
            .expect("ohlcv");

        let vwap = bar.vwap_proxy();
        let sma_50 = self.sma_50.next(bar.close);
        let sma_200 = self.sma_200.next(bar.close);
        let rsi_14 = self.rsi_14.next(bar.close);
        let macd_out = self.macd.next(bar.close);
        let bb_out = self.bb.next(bar.close);
        let atr_14 = self.atr_14.next(&item);

        let log_ret = match self.prev_close {
            Some(pc) if pc > 0.0 && bar.close > 0.0 => (bar.close / pc).ln(),
            _ => f64::NAN,
        };

        let dist_sma50 = if sma_50.abs() > f64::EPSILON {
            (bar.close - sma_50) / sma_50
        } else {
            f64::NAN
        };
        let dist_sma200 = if sma_200.abs() > f64::EPSILON {
            (bar.close - sma_200) / sma_200
        } else {
            f64::NAN
        };

        let bb_width = if sma_50.abs() > f64::EPSILON {
            (bb_out.upper - bb_out.lower) / sma_50
        } else {
            f64::NAN
        };

        let spread = bb_out.upper - bb_out.lower;
        let bb_position = if spread.abs() > f64::EPSILON {
            (bar.close - bb_out.lower) / spread
        } else {
            f64::NAN
        };

        let macd_hist = macd_out.macd - macd_out.signal;
        let atr_norm = if bar.close.abs() > f64::EPSILON {
            atr_14 / bar.close
        } else {
            f64::NAN
        };

        let vol_change = match self.prev_vol {
            Some(pv) if pv > 0.0 => (bar.volume - pv) / pv,
            _ => 0.0,
        };

        self.prev_close = Some(bar.close);
        self.prev_vol = Some(bar.volume);

        ShunyaFeatureRow {
            vwap,
            sma_50,
            sma_200,
            rsi_14,
            macd: macd_out.macd,
            macd_signal: macd_out.signal,
            bb_upper: bb_out.upper,
            bb_lower: bb_out.lower,
            atr_14,
            log_ret,
            dist_sma50,
            dist_sma200,
            bb_width,
            bb_position,
            macd_hist,
            atr_norm,
            vol_change,
        }
    }
}

/// Run [`ShunyaFeatureEngine`] over a slice (convenience).
pub fn features_from_bars(bars: &[OhlcvBar]) -> Result<Vec<ShunyaFeatureRow>, ta::errors::TaError> {
    let mut eng = ShunyaFeatureEngine::new()?;
    Ok(bars.iter().map(|b| eng.push(b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_uptrend(n: usize) -> Vec<OhlcvBar> {
        let t0: chrono::DateTime<chrono::Utc> = "2020-01-01T00:00:00Z".parse().unwrap();
        (0..n)
            .map(|i| {
                let p = 100.0 + i as f64 * 0.1;
                OhlcvBar {
                    time: t0 + chrono::Duration::days(i as i64),
                    open: p,
                    high: p + 0.05,
                    low: p - 0.05,
                    close: p,
                    volume: 1_000_000.0 + i as f64 * 100.0,
                }
            })
            .collect()
    }

    #[test]
    fn last_row_finite_after_warmup() {
        let bars = synth_uptrend(250);
        let rows = features_from_bars(&bars).unwrap();
        let last = rows.last().unwrap();
        assert!(last.sma_50.is_finite());
        assert!(last.sma_200.is_finite());
        assert!(last.rsi_14.is_finite());
        assert!(last.macd.is_finite());
        assert!(last.bb_upper.is_finite());
        assert!(last.atr_14.is_finite());
        assert!(last.log_ret.is_finite());
    }

    #[test]
    fn snapshot_deterministic_tiny_series() {
        let bars = vec![
            OhlcvBar {
                time: "2020-01-01T00:00:00Z".parse().unwrap(),
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.0,
                volume: 1000.0,
            },
            OhlcvBar {
                time: "2020-01-02T00:00:00Z".parse().unwrap(),
                open: 10.0,
                high: 12.0,
                low: 10.0,
                close: 11.0,
                volume: 1100.0,
            },
            OhlcvBar {
                time: "2020-01-03T00:00:00Z".parse().unwrap(),
                open: 11.0,
                high: 11.5,
                low: 10.5,
                close: 11.2,
                volume: 900.0,
            },
        ];
        let rows = features_from_bars(&bars).unwrap();
        assert_eq!(rows.len(), 3);
        assert!((rows[0].vwap - 10.0).abs() < 1e-9);
        let exp_sma50 = (10.0_f64 + 11.0 + 11.2) / 3.0;
        assert!(
            (rows[2].sma_50 - exp_sma50).abs() < 1e-9,
            "sma50={} expected {}",
            rows[2].sma_50,
            exp_sma50
        );
    }
}

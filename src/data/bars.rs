//! Alpaca stock bars (REST): pagination helper → [`super::candle::OhlcvBar`].

use crate::data::candle::OhlcvBar;
use apca::Client;
use apca::RequestError;
use apca::data::v2::bars::{List, ListError, ListReq, ListReqInit};

/// Fetch all pages of daily (or any) bars for one symbol, oldest-first within each page;
/// results are concatenated in API order (concatenate pages as returned).
pub async fn fetch_stock_bars(
    client: &Client,
    mut req: ListReq,
) -> Result<Vec<OhlcvBar>, RequestError<ListError>> {
    let mut out: Vec<OhlcvBar> = Vec::new();
    loop {
        let page = client.issue::<List>(&req).await?;
        out.extend(page.bars.iter().map(OhlcvBar::from));
        match page.next_page_token {
            Some(token) => req.page_token = Some(token),
            None => break,
        }
    }
    Ok(out)
}

/// Build a [`ListReq`] with defaults (no adjustment override).
pub fn list_req(
    symbol: impl Into<String>,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
    timeframe: apca::data::v2::bars::TimeFrame,
) -> ListReq {
    ListReqInit::default().init(symbol, start, end, timeframe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apca::data::v2::bars::Bar as ApcaBar;
    use chrono::DateTime;
    use num_decimal::Num;

    #[test]
    fn ohlcv_from_rest_bar() {
        let t: DateTime<chrono::Utc> = "2021-02-01T16:01:00Z".parse().unwrap();
        let b = ApcaBar {
            time: t,
            open: Num::new(13332, 100),
            close: Num::new(1335, 10),
            high: Num::new(13374, 100),
            low: Num::new(13331, 100),
            volume: 9876,
            weighted_average: Num::new(1334, 10),
            _non_exhaustive: (),
        };
        let o = OhlcvBar::from(&b);
        assert!((o.open - 133.32).abs() < 1e-6);
        assert!((o.close - 133.5).abs() < 1e-6);
        assert_eq!(o.volume, 9876.0);
    }
}

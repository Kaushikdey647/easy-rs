//! Lee–Ready style trade classification using last NBBO (no aggressor flag on Alpaca stream trades).

/// Signed share count: buyer-initiated positive, seller-initiated negative.
///
/// `last_trade_px_ticks` is the previous trade price in the same tick space as `trade_px_ticks`.
/// Returns updated last trade price (always `Some(trade_px_ticks)` after a classified trade).
#[inline]
pub fn lee_ready_signed_volume(
    trade_px_ticks: u64,
    trade_sz: u64,
    bid_ticks: u64,
    ask_ticks: u64,
    last_trade_px: Option<u64>,
) -> (i64, Option<u64>) {
    if trade_sz == 0 {
        return (0, last_trade_px);
    }
    if bid_ticks >= ask_ticks {
        // Crossed or invalid NBBO — skip classification.
        return (0, last_trade_px);
    }
    let mid = bid_ticks.saturating_add(ask_ticks) / 2;
    let sz = trade_sz as i64;
    let signed = if trade_px_ticks > mid {
        sz
    } else if trade_px_ticks < mid {
        -sz
    } else {
        tick_test(trade_px_ticks, sz, last_trade_px)
    };
    (signed, Some(trade_px_ticks))
}

#[inline]
fn tick_test(trade_px_ticks: u64, sz: i64, last_trade_px: Option<u64>) -> i64 {
    let Some(prev) = last_trade_px else {
        return 0;
    };
    if trade_px_ticks > prev {
        sz
    } else if trade_px_ticks < prev {
        -sz
    } else {
        0
    }
}

/// Map a price to a row index in `[0, rows)` given linear bounds.
#[inline]
pub fn price_to_row(price: f64, p_min: f64, p_max: f64, rows: usize) -> usize {
    if rows == 0 {
        return 0;
    }
    let span = (p_max - p_min).max(f64::EPSILON * 64.0);
    let t = ((price - p_min) / span).clamp(0.0, 1.0);
    let idx = (t * (rows.saturating_sub(1)) as f64).round() as usize;
    idx.min(rows - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_ready_above_mid_buy() {
        // mid = (100+198)/2 = 149; trade above mid => buy
        let (d, last) = lee_ready_signed_volume(150, 10, 100, 198, None);
        assert_eq!(d, 10);
        assert_eq!(last, Some(150));
    }

    #[test]
    fn lee_ready_below_mid_sell() {
        let (d, last) = lee_ready_signed_volume(90, 5, 100, 200, None);
        assert_eq!(d, -5);
        assert_eq!(last, Some(90));
    }

    #[test]
    fn lee_ready_at_mid_tick_up() {
        let (d, last) = lee_ready_signed_volume(150, 7, 100, 200, Some(140));
        assert_eq!(d, 7);
        assert_eq!(last, Some(150));
    }

    #[test]
    fn lee_ready_at_mid_tick_down() {
        let (d, last) = lee_ready_signed_volume(150, 7, 100, 200, Some(160));
        assert_eq!(d, -7);
        assert_eq!(last, Some(150));
    }

    #[test]
    fn lee_ready_at_mid_no_prev() {
        let (d, last) = lee_ready_signed_volume(150, 7, 100, 200, None);
        assert_eq!(d, 0);
        assert_eq!(last, Some(150));
    }

    #[test]
    fn inverted_book_skips() {
        let (d, last) = lee_ready_signed_volume(150, 10, 200, 100, None);
        assert_eq!(d, 0);
        assert_eq!(last, None);
    }

    #[test]
    fn price_row_endpoints() {
        assert_eq!(price_to_row(0.0, 0.0, 10.0, 5), 0);
        assert_eq!(price_to_row(10.0, 0.0, 10.0, 5), 4);
        assert_eq!(price_to_row(5.0, 0.0, 10.0, 5), 2);
    }
}

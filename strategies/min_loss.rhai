// name: "sma_cross_state"
// Typed Runtime SMA crossover strategy with primitive Strategy State.
// Tracks the highest close seen while long and closes the long if price falls
// more than TRAIL_PCT from that session-local peak.

const FAST = 20;
const SLOW = 50;
const QUANTITY = 1.0;
const TRAIL_PCT = 0.05;

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1d"))
        .with_minimum_warmup(SLOW + 1)
}

fn on_tick(market, context) {
    let candles = market.candles();
    let current = market.candle();
    let position = context.portfolio.position;

    if position != () && position.side == "Long" {
        let peak = context.state.get("peak_close", current.close);
        let next_peak = if current.close > peak { current.close } else { peak };
        context.state.set("peak_close", next_peak);

        if current.close < next_peak * (1.0 - TRAIL_PCT) {
            return decision::close_long().with_reason("trailing close below peak");
        }
    } else {
        context.state.set("peak_close", current.close);
    }

    let fast      = indicators::sma(candles, FAST);
    let slow      = indicators::sma(candles, SLOW);
    let fast_prev = indicators::sma(candles, FAST, 1);
    let slow_prev = indicators::sma(candles, SLOW, 1);

    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return decision::hold().with_reason("warming up");
    }

    let crossed_up   = fast_prev <= slow_prev && fast > slow;
    let crossed_down = fast_prev >= slow_prev && fast < slow;

    if crossed_up && position == () {
        context.state.set("peak_close", current.close);
        return decision::open_long(QUANTITY).with_reason("fast SMA crossed above slow SMA");
    }

    if crossed_down && position != () && position.side == "Long" {
        return decision::close_long().with_reason("fast SMA crossed below slow SMA");
    }

    decision::hold()
}

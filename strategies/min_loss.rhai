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

    if position != () && position.is_long() {
        let peak = context.state.float("peak_close", current.close);
        let next_peak = if current.close > peak { current.close } else { peak };
        context.state.set_float("peak_close", next_peak);

        if current.close < next_peak * (1.0 - TRAIL_PCT) {
            return decision::close_long().with_reason("trailing close below peak");
        }
    } else {
        context.state.set_float("peak_close", current.close);
    }

    let fast      = ta::sma(candles, FAST);
    let slow      = ta::sma(candles, SLOW);
    let fast_prev = ta::sma(candles, FAST, 1);
    let slow_prev = ta::sma(candles, SLOW, 1);

    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return decision::hold().with_reason("warming up");
    }

    let crossed_up   = ta::cross_over(fast_prev, slow_prev, fast, slow);
    let crossed_down = ta::cross_under(fast_prev, slow_prev, fast, slow);

    if crossed_up && context.portfolio.is_flat() {
        context.state.set_float("peak_close", current.close);
        return decision::open_long(QUANTITY).with_reason("fast SMA crossed above slow SMA");
    }

    if crossed_down && position != () && position.is_long() {
        return decision::close_long().with_reason("fast SMA crossed below slow SMA");
    }

    decision::hold()
}

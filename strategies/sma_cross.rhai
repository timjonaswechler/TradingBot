// name: "sma_cross"
// SMA Crossover Strategy
// BUY  when the fast SMA crosses above the slow SMA
// SELL when the fast SMA crosses below the slow SMA

const FAST = 50;
const SLOW = 200;

fn on_tick(candles, context) {
    let fast      = indicators::sma(candles, FAST);
    let slow      = indicators::sma(candles, SLOW);
    let fast_prev = indicators::sma(candles, FAST, 1);
    let slow_prev = indicators::sma(candles, SLOW, 1);

    // Not enough data yet
    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return #{ signal: "HOLD", reason: "warming up" };
    }

    let crossed_up   = fast_prev <= slow_prev && fast > slow;
    let crossed_down = fast_prev >= slow_prev && fast < slow;

    if crossed_up && context.position == () {
        return #{
            signal:      "BUY",
            size:        0.5,
            stop_loss:   candles[1].low * 0.98,
            take_profit: candles[1].close * 1.10,
            reason:      `SMA${FAST} crossed above SMA${SLOW}`,
        };
    }

    if crossed_down && context.position != () {
        return #{
            signal: "SELL",
            reason: `SMA${FAST} crossed below SMA${SLOW}`,
        };
    }

    #{ signal: "HOLD" }
}

-- SMA Crossover Strategy
-- BUY  when fast SMA crosses above slow SMA
-- SELL when fast SMA crosses below slow SMA

local config = { name = "sma_cross" }

local FAST = 10
local SLOW = 30

function on_tick(candles, context)
    local fast     = indicators.sma(candles, FAST)
    local slow     = indicators.sma(candles, SLOW)
    local fast_prev = indicators.sma(candles, FAST, 1)
    local slow_prev = indicators.sma(candles, SLOW, 1)

    -- Not enough data yet
    if fast == nil or slow == nil or fast_prev == nil or slow_prev == nil then
        return { signal = "HOLD", reason = "warming up" }
    end

    local crossed_up   = fast_prev <= slow_prev and fast > slow
    local crossed_down = fast_prev >= slow_prev and fast < slow

    if crossed_up and context.position == nil then
        return {
            signal      = "BUY",
            size        = 0.5,
            stop_loss   = candles[1].low * 0.98,
            take_profit = candles[1].close * 1.10,
            reason      = string.format("SMA%d crossed above SMA%d", FAST, SLOW),
        }
    end

    if crossed_down and context.position ~= nil then
        return {
            signal = "SELL",
            reason = string.format("SMA%d crossed below SMA%d", FAST, SLOW),
        }
    end

    return { signal = "HOLD" }
end

return config

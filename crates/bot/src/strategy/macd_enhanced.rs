use super::{Signal, Strategy};
use crate::market_data::Candle;

/// Alle tunable Parameter der Enhanced-MACD-Strategie.
/// fast/slow/signal_period sind FEST (werden vom Optimizer nicht geändert).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct MacdEnhancedParams {
    // ── Fest (vom Nutzer in config.toml gesetzt) ─────────────────────────────
    pub fast_period:   usize, // typisch 12
    pub slow_period:   usize, // typisch 26
    pub signal_period: usize, // typisch 9

    // ── K1: Kreuzungsrichtung ─────────────────────────────────────────────────
    /// Gewicht des klassischen MACD/Signal-Crossovers
    pub crossover_weight: f64, // [0.0, 3.0]

    // ── K2: Nulllinien-Lage ───────────────────────────────────────────────────
    /// Gewicht: Buy-Kreuzung unter Null = stärker, Sell über Null = stärker
    pub zero_line_weight: f64,  // [0.0, 3.0]
    /// Totzone um Null: Kreuzungen innerhalb ±deadband gelten als neutral
    /// (normiert als Bruchteil des Preises, z.B. 0.001 = 0,1%)
    pub zero_line_deadband: f64, // [0.0, 0.01]

    // ── K3: Histogramm-Stärke ─────────────────────────────────────────────────
    /// Gewicht: großes Histogramm = starker Impuls
    pub histogram_strength_weight: f64, // [0.0, 3.0]
    /// Mindeststärke: darunter zählt das Kreuzungssignal nicht (normiert)
    pub histogram_min_threshold: f64,   // [0.0, 0.005]

    // ── K4: Histogramm-Momentum ───────────────────────────────────────────────
    /// Gewicht: wächst das Histogramm oder schrumpft es?
    pub histogram_momentum_weight: f64, // [0.0, 3.0]
    /// Über wie viele Bars wird das Histogramm-Momentum gemessen
    pub histogram_lookback: usize,      // [1, 5]

    // ── K5: Histogramm-Wendepunkt (Frühsignal) ────────────────────────────────
    /// Gewicht: Histogramm dreht vor der Kreuzung → frühes Signal
    pub histogram_reversal_weight: f64, // [0.0, 3.0]
    /// Wie viele Bars muss die Umkehr bestätigt sein
    pub reversal_confirm_bars: usize,   // [1, 3]

    // ── K6: MACD-Linien-Steigung ──────────────────────────────────────────────
    /// Gewicht: wie steil steigt/fällt die MACD-Linie selbst?
    pub macd_slope_weight: f64, // [0.0, 3.0]
    /// Über wie viele Bars wird die Steigung berechnet (geteilt mit K7/K8)
    pub slope_lookback: usize,  // [1, 10]

    // ── K7: EMA-Fast Steigung (Kursplot) ──────────────────────────────────────
    /// Gewicht der Fast-EMA-Steigung auf dem absoluten Kursplot
    pub ema_fast_slope_weight: f64, // [0.0, 3.0]

    // ── K8: EMA-Slow Steigung (Kursplot) ──────────────────────────────────────
    /// Gewicht der Slow-EMA-Steigung auf dem absoluten Kursplot
    pub ema_slow_slope_weight: f64, // [0.0, 3.0]

    // ── K9: Gleichlauf-Filter ─────────────────────────────────────────────────
    /// Stärke des Filters: wenn beide EMAs stark gleichgerichtet laufen,
    /// wird ein Gegensignal abgemildert
    pub trend_filter_weight: f64,    // [0.0, 3.0]
    /// Ab dieser Steigung (normiert, % pro Bar) greift der Filter
    pub trend_filter_min_slope: f64, // [0.0, 0.005]

    // ── K10: EMA-Abstand (Spreizung) ──────────────────────────────────────────
    /// Gewicht: großer EMA-Abstand = starker Trend
    pub ema_separation_weight: f64, // [0.0, 3.0]

    // ── K11: Trend-Regime (langer EMA) ────────────────────────────────────────
    /// Periode des langen EMA der das Markt-Regime bestimmt (z.B. 50 oder 200)
    pub regime_period: usize,   // [20, 200]
    /// Gewicht: wie stark sperrt/öffnet das Regime den Score?
    /// Hoch (z.B. 3.0) → wirkt wie ein Hard-Gate (Score zu klein wenn gegen Trend)
    pub regime_weight: f64,     // [0.0, 5.0]
    /// Totzone: Preis muss mindestens diesen Abstand (% des Preises) vom langen EMA haben
    /// damit das Regime zählt. Im Bereich ±deadband → neutral (keine Aktie)
    pub regime_deadband: f64,   // [0.0, 0.05]

    // ── Exit-MACD (schneller MACD als frühzeitiger Exit) ──────────────────────
    /// Schnelle Periode des Exit-MACD (kleiner als fast_period für früheren Exit)
    pub exit_fast_period:   usize, // [2, 12]
    /// Langsame Periode des Exit-MACD
    pub exit_slow_period:   usize, // [5, 26]
    /// Signal-Periode des Exit-MACD
    pub exit_signal_period: usize, // [2, 9]
    /// Gewicht: wie stark zieht ein bärischer Exit-MACD den Score nach unten?
    /// Hoch (z.B. 3.0) → wirkt wie ein Hard-Exit sobald Exit-MACD dreht
    pub exit_weight: f64, // [0.0, 5.0]

    // ── Entscheidungs-Schwellen ────────────────────────────────────────────────
    /// Gesamtscore muss diesen Wert überschreiten → BUY
    pub buy_threshold: f64,  // [0.0, 5.0]
    /// Gesamtscore muss diesen Wert unterschreiten (negativ) → SELL
    pub sell_threshold: f64, // [0.0, 5.0]
}

impl Default for MacdEnhancedParams {
    fn default() -> Self {
        Self {
            fast_period:   12,
            slow_period:   26,
            signal_period: 9,
            // Klassische Kreuzung als Hauptsignal, alles andere moderat
            crossover_weight:           2.0,
            zero_line_weight:           0.5,
            zero_line_deadband:         0.0,
            histogram_strength_weight:  0.5,
            histogram_min_threshold:    0.0,
            histogram_momentum_weight:  0.3,
            histogram_lookback:         2,
            histogram_reversal_weight:  0.3,
            reversal_confirm_bars:      1,
            macd_slope_weight:          0.4,
            slope_lookback:             3,
            ema_fast_slope_weight:      0.3,
            ema_slow_slope_weight:      0.2,
            trend_filter_weight:        0.5,
            trend_filter_min_slope:     0.0005,
            ema_separation_weight:      0.2,
            regime_period:              50,
            regime_weight:              2.0,
            regime_deadband:            0.005,
            exit_fast_period:           5,
            exit_slow_period:           13,
            exit_signal_period:         4,
            exit_weight:                2.0,
            buy_threshold:              1.0,
            sell_threshold:             1.0,
        }
    }
}

pub struct MacdEnhanced {
    pub params: MacdEnhancedParams,
}

impl MacdEnhanced {
    pub fn new(params: MacdEnhancedParams) -> Self {
        Self { params }
    }
}

// ── Interner Datentyp für einen MACD-Messpunkt ────────────────────────────────

struct MacdPoint {
    macd_line:   f64, // (EMA_fast − EMA_slow) / Preis
    signal_line: f64, // EMA(macd_series) / Preis
    histogram:   f64, // macd_line − signal_line
}

// ── Strategy-Implementierung ──────────────────────────────────────────────────

impl Strategy for MacdEnhanced {
    fn name(&self) -> &str { "MACD Enhanced" }

    fn required_history(&self) -> usize {
        let p = &self.params;
        // slow EMA warmup + signal EMA + genug für Slopes und Reversal-Erkennung
        let extra = p.slope_lookback
            .max(p.histogram_lookback)
            .max(p.reversal_confirm_bars + 1);
        let macd_needs = p.slow_period + p.signal_period + extra + 2;
        // K11: langer EMA braucht entsprechend viele Bars
        // Exit-MACD: eigener Warmup
        let exit_needs = p.exit_slow_period + p.exit_signal_period + 2;
        macd_needs.max(p.regime_period).max(exit_needs)
    }

    fn signal(&self, candles: &[Candle]) -> Signal {
        let score = self.compute_score(candles);
        let p = &self.params;
        if score > p.buy_threshold {
            Signal::Buy
        } else if score < -p.sell_threshold {
            Signal::Sell
        } else {
            Signal::Hold
        }
    }
}

impl MacdEnhanced {
    fn compute_score(&self, candles: &[Candle]) -> f64 {
        let p = &self.params;
        let price = candles[0].close as f64;
        if price <= 0.0 {
            return 0.0;
        }

        // Wie viele MACD-Punkte brauchen wir für Slopes + Reversal?
        let series_len = p.slope_lookback
            .max(p.histogram_lookback)
            .max(p.reversal_confirm_bars + 1)
            + 2; // +2: heute + gestern für Crossover-Vergleich

        let pts = compute_macd_series(candles, p.fast_period, p.slow_period, p.signal_period, series_len);
        if pts.len() < 2 {
            return 0.0;
        }

        let today = &pts[0];
        let prev  = &pts[1];

        // ── K1: Kreuzungsrichtung ─────────────────────────────────────────────
        let crossover_dir = if prev.macd_line <= prev.signal_line && today.macd_line > today.signal_line {
            1.0   // bullisch
        } else if prev.macd_line >= prev.signal_line && today.macd_line < today.signal_line {
            -1.0  // bärisch
        } else {
            0.0
        };
        let mut score = crossover_dir * p.crossover_weight;

        // ── K2: Nulllinien-Lage ───────────────────────────────────────────────
        // Buy-Kreuzung unter Null → starkes Signal (+), über Null → schwach (−)
        // Sell-Kreuzung über Null → stark, unter Null → schwach
        if crossover_dir != 0.0 {
            let macd_norm = today.macd_line; // bereits normiert durch compute_macd_series
            if macd_norm.abs() > p.zero_line_deadband {
                // Wenn Kreuzungsrichtung und Nulllinien-Seite übereinstimmen → Bonus
                // Buy (+ 1) + MACD < 0 → favorable → +bonus
                // Sell (−1) + MACD > 0 → favorable → −bonus
                let favorable = crossover_dir * macd_norm.signum() < 0.0; // entgegengesetzt = gut
                let bonus = if favorable { 1.0 } else { -0.5 };
                score += bonus * p.zero_line_weight;
            }
            // Im Deadband: kein Beitrag (neutrales Gebiet)
        }

        // ── K3: Histogramm-Stärke ─────────────────────────────────────────────
        let hist_abs = today.histogram.abs();
        if hist_abs >= p.histogram_min_threshold {
            // Richtung des Histogramms mit Kreuzungsrichtung vergleichen
            let hist_aligned = (today.histogram * crossover_dir) > 0.0;
            let strength_contrib = if hist_aligned { hist_abs } else { -hist_abs * 0.5 };
            score += strength_contrib * p.histogram_strength_weight;
        }

        // ── K4: Histogramm-Momentum ───────────────────────────────────────────
        if pts.len() > p.histogram_lookback {
            let oldest_hist = pts[p.histogram_lookback].histogram;
            let hist_delta = today.histogram - oldest_hist;
            // Normiert: wie schnell ändert sich das Histogramm?
            score += hist_delta * p.histogram_momentum_weight;
        }

        // ── K5: Histogramm-Wendepunkt (Frühsignal) ────────────────────────────
        // Hat das Histogramm in den letzten `reversal_confirm_bars` Bars gedreht?
        if pts.len() > p.reversal_confirm_bars + 1 {
            let reversal = detect_histogram_reversal(&pts, p.reversal_confirm_bars);
            score += reversal * p.histogram_reversal_weight;
        }

        // ── K6: MACD-Linien-Steigung ──────────────────────────────────────────
        if pts.len() > p.slope_lookback {
            let macd_values: Vec<f64> = pts[..=p.slope_lookback].iter()
                .map(|pt| pt.macd_line)
                .collect();
            let macd_slope = linear_slope(&macd_values); // normiert, neueste zuerst
            score += macd_slope * p.macd_slope_weight;
        }

        // ── K7 & K8: EMA-Steigungen aus dem Kursplot ─────────────────────────
        let fast_slope = ema_slope(candles, p.fast_period, p.slope_lookback, price);
        let slow_slope = ema_slope(candles, p.slow_period, p.slope_lookback, price);
        score += fast_slope * p.ema_fast_slope_weight;
        score += slow_slope * p.ema_slow_slope_weight;

        // ── K9: Gleichlauf-Filter ─────────────────────────────────────────────
        // Wenn beide EMA-Steigungen stark gleichgerichtet sind und das Signal
        // dagegen läuft → Abzug
        let avg_slope = (fast_slope + slow_slope) / 2.0;
        if avg_slope.abs() > p.trend_filter_min_slope {
            let against_trend = (avg_slope * crossover_dir) < 0.0;
            if against_trend {
                score -= avg_slope.abs() * p.trend_filter_weight;
            }
        }

        // ── K10: EMA-Abstand (Spreizung) ──────────────────────────────────────
        // Abstand zwischen Fast und Slow EMA, normiert durch den Preis
        if candles.len() > p.slow_period {
            let fast_ema = ema_at_offset(candles, 0, p.fast_period);
            let slow_ema = ema_at_offset(candles, 0, p.slow_period);
            let separation = (fast_ema - slow_ema) / price;
            score += separation * p.ema_separation_weight;
        }

        // ── K11: Trend-Regime (langer EMA) ────────────────────────────────────
        // Preis über langem EMA  → bullisches Regime → Buy-Score aufwerten
        // Preis unter langem EMA → bärisches Regime  → Score abwerten (Gate)
        // Preis nahe EMA (±deadband) → Seitwärts → neutral, kein Beitrag
        //
        // Bei passiven Aktien (KO, JNJ, PG) liegt der Kurs oft jahrelang nahe
        // dem langen EMA → wenig Beitrag → Buy-Schwelle wird selten erreicht.
        // Bei Momentum-Aktien (TSLA, NVDA) im Uptrend → klarer positiver Beitrag.
        if candles.len() >= p.regime_period {
            let regime_ema = ema_at_offset(candles, 0, p.regime_period);
            let regime_dist = (price - regime_ema) / price; // normiert, ± %
            if regime_dist.abs() > p.regime_deadband {
                score += regime_dist.signum() * p.regime_weight;
            }
        }

        // ── Exit-MACD: schneller MACD als frühzeitiger Exit ───────────────────
        // Wenn der schnelle MACD bärisch ist (Linie < Signal), wird der Score
        // nach unten gezogen → sell_threshold wird früher erreicht.
        // Kein Effekt auf positive Scores (verhindert nicht den Einstieg).
        if p.exit_weight > 0.0 {
            let exit_pts = compute_macd_series(
                candles,
                p.exit_fast_period,
                p.exit_slow_period,
                p.exit_signal_period,
                2,
            );
            if exit_pts.len() >= 1 {
                let exit_today = &exit_pts[0];
                if exit_today.macd_line < exit_today.signal_line {
                    // Bärisch → Score nach unten ziehen
                    score -= p.exit_weight;
                }
            }
        }

        score
    }
}

// ── Hilfsfunktionen ───────────────────────────────────────────────────────────

/// Berechnet `count` MACD-Messpunkte (neueste zuerst), normiert durch candles[0].close.
/// Jeder Punkt: macd_line=(EMA_fast−EMA_slow)/Preis, signal=EMA(macd_serie)/Preis.
fn compute_macd_series(
    candles:       &[Candle],
    fast:          usize,
    slow:          usize,
    signal_period: usize,
    count:         usize,
) -> Vec<MacdPoint> {
    // Für jeden der `count` Zeitpunkte (offset 0 = heute) brauchen wir
    // `signal_period` weitere MACD-Rohwerte zurück für die Signal-EMA.
    let raw_needed = count + signal_period;
    let mut raw_macd: Vec<f64> = Vec::with_capacity(raw_needed);

    for offset in 0..raw_needed {
        let slice = &candles[offset..];
        if slice.len() < slow {
            break;
        }
        let f = ema_candles(slice, fast);
        let s = ema_candles(slice, slow);
        raw_macd.push(f - s); // neueste zuerst
    }

    if raw_macd.len() < signal_period + 1 {
        return vec![];
    }

    let price = candles[0].close as f64;
    let norm = if price > 0.0 { 1.0 / price } else { 1.0 };

    let actual = raw_macd.len().saturating_sub(signal_period - 1).min(count);
    let mut pts = Vec::with_capacity(actual);

    for i in 0..actual {
        // Signal-Linie bei Offset i = EMA der MACD-Werte [i .. i+signal_period]
        let sig_slice = &raw_macd[i..i + signal_period];
        let sig_val   = ema_of_slice_newest_first(sig_slice);
        pts.push(MacdPoint {
            macd_line:   raw_macd[i] * norm,
            signal_line: sig_val * norm,
            histogram:   (raw_macd[i] - sig_val) * norm,
        });
    }
    pts
}

/// EMA von `period` Candles ab `offset` (neueste zuerst in `candles`).
fn ema_at_offset(candles: &[Candle], offset: usize, period: usize) -> f64 {
    let slice = &candles[offset..];
    if slice.len() < period { return 0.0; }
    ema_candles(slice, period)
}

/// EMA der ersten `period` Candles (neueste zuerst).
fn ema_candles(candles: &[Candle], period: usize) -> f64 {
    if candles.len() < period { return 0.0; }
    let prices: Vec<f64> = candles.iter().map(|c| c.close as f64).collect();
    // Von alt→neu sortieren für korrekte EMA-Berechnung
    let oldest_first: Vec<f64> = prices[..candles.len()].iter().rev().cloned().collect();
    ema_from_oldest(&oldest_first, period)
}

/// EMA aus aufsteigend sortierten Werten (älteste zuerst).
fn ema_from_oldest(values: &[f64], period: usize) -> f64 {
    if values.len() < period { return 0.0; }
    let k = 2.0 / (period as f64 + 1.0);
    let mut val = values[..period].iter().sum::<f64>() / period as f64;
    for &v in &values[period..] {
        val = v * k + val * (1.0 - k);
    }
    val
}

/// EMA aus neueste-zuerst sortierten Werten (umkehren intern).
fn ema_of_slice_newest_first(values: &[f64]) -> f64 {
    let rev: Vec<f64> = values.iter().rev().cloned().collect();
    ema_from_oldest(&rev, values.len())
}

/// Berechnet die Steigung einer Zeitreihe (neueste zuerst) via einfacher
/// linearer Regression. Gibt den Anstieg pro Bar zurück (normiert auf Werte ≈ [-1, 1]).
fn linear_slope(values: &[f64]) -> f64 {
    let n = values.len();
    if n < 2 { return 0.0; }
    // x: 0 = älteste (index n-1), n-1 = neueste (index 0)
    let mut sum_x = 0.0_f64;
    let mut sum_y = 0.0_f64;
    let mut sum_xy = 0.0_f64;
    let mut sum_xx = 0.0_f64;
    for i in 0..n {
        let x = (n - 1 - i) as f64; // neueste = höchster x-Wert
        let y = values[i];
        sum_x  += x;
        sum_y  += y;
        sum_xy += x * y;
        sum_xx += x * x;
    }
    let fn_ = n as f64;
    let denom = fn_ * sum_xx - sum_x * sum_x;
    if denom.abs() < 1e-12 { return 0.0; }
    (fn_ * sum_xy - sum_x * sum_y) / denom
}

/// Steigung der EMA(period) über `lookback` Bars (normiert durch aktuellen Preis).
fn ema_slope(candles: &[Candle], period: usize, lookback: usize, price: f64) -> f64 {
    if candles.len() < period + lookback + 1 || price <= 0.0 {
        return 0.0;
    }
    let ema_vals: Vec<f64> = (0..=lookback)
        .map(|offset| ema_at_offset(candles, offset, period) / price)
        .collect();
    linear_slope(&ema_vals)
}

/// Erkennt ob das Histogramm in den letzten `confirm_bars` einen Wendepunkt hat.
/// Gibt +1.0 (bullischer Wendepunkt: Hist war negativ und dreht nach oben)
/// oder −1.0 (bärischer Wendepunkt) zurück, sonst 0.0.
fn detect_histogram_reversal(pts: &[MacdPoint], confirm_bars: usize) -> f64 {
    if pts.len() < confirm_bars + 2 { return 0.0; }

    // Prüfe ob Histogramm in den letzten `confirm_bars` monoton in eine Richtung
    // und jetzt dreht
    let oldest_in_window = confirm_bars + 1;
    let base_hist = pts[oldest_in_window].histogram;

    // Alle Bars im Fenster (ausser heute) zeigen in dieselbe Richtung?
    let all_same_direction = (1..=confirm_bars).all(|i| {
        (pts[i].histogram - base_hist) * (pts[confirm_bars].histogram - base_hist) >= 0.0
    });

    if !all_same_direction { return 0.0; }

    let trend_was_down = pts[confirm_bars].histogram < base_hist;
    let today_reverses = if trend_was_down {
        pts[0].histogram > pts[1].histogram
    } else {
        pts[0].histogram < pts[1].histogram
    };

    if today_reverses {
        if trend_was_down { 1.0 } else { -1.0 }
    } else {
        0.0
    }
}

use anyhow::Result;
use bot::{
    config,
    db::Db,
    market_data::Candle,
    strategy::{self, Signal, Strategy},
};
use chrono::NaiveDate;
use plotters::prelude::*;
use std::path::Path;

fn main() -> Result<()> {
    let cfg = config::Config::load(Path::new("config.toml"))?;
    let db  = Db::open(&cfg.db.path)?;

    // Asset aus Argument oder erstes aus Watchlist
    let asset = std::env::args()
        .skip_while(|a| a != "--asset")
        .nth(1)
        .unwrap_or_else(|| {
            cfg.assets.watchlist.first().cloned().unwrap_or_else(|| "AAPL".into())
        });

    let strat: Box<dyn Strategy> = strategy::from_config(&cfg.strategy)?;

    let candles = db.get_all_candles_asc(&asset, cfg.data.primary_interval())?;
    let h = strat.required_history();
    anyhow::ensure!(
        candles.len() >= h,
        "Nicht genug Daten für {asset} – erst `cargo run -p bot` ausführen"
    );

    // ── SMA-Linien ────────────────────────────────────────────────────────────
    let short_sma = sma_series(&candles, cfg.strategy.short_period);
    let long_sma  = sma_series(&candles, cfg.strategy.long_period);

    // ── Buy/Sell-Signale ──────────────────────────────────────────────────────
    let mut buys:  Vec<(NaiveDate, f64)> = Vec::new();
    let mut sells: Vec<(NaiveDate, f64)> = Vec::new();

    for t in (h - 1)..candles.len() {
        let window: Vec<_> = candles[t + 1 - h..=t].iter().rev().cloned().collect();
        let date  = candles[t].timestamp.date_naive();
        let price = candles[t].close as f64 / 100.0;
        match strat.signal(&window) {
            Signal::Buy  => buys.push((date, price)),
            Signal::Sell => sells.push((date, price)),
            Signal::Hold => {}
        }
    }

    // ── Chart-Grenzen ─────────────────────────────────────────────────────────
    let dates:  Vec<NaiveDate> = candles.iter().map(|c| c.timestamp.date_naive()).collect();
    let prices: Vec<f64>       = candles.iter().map(|c| c.close as f64 / 100.0).collect();

    let min_p = prices.iter().cloned().fold(f64::INFINITY,     f64::min) * 0.96;
    let max_p = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * 1.04;

    // ── Ausgabe-Datei ─────────────────────────────────────────────────────────
    let out = format!("{}_chart.png", asset.to_lowercase().replace(['^', '/'], "_"));
    let root = BitMapBackend::new(&out, (1500, 750)).into_drawing_area();
    root.fill(&RGBColor(18, 18, 28))?;

    let mut chart = ChartBuilder::on(&root)
        .caption(
            format!(
                "{asset}  ·  SMA {}/{}  ·  {} – {}",
                cfg.strategy.short_period,
                cfg.strategy.long_period,
                dates.first().unwrap(),
                dates.last().unwrap(),
            ),
            ("sans-serif", 20).into_font().color(&RGBColor(210, 210, 220)),
        )
        .margin(35)
        .x_label_area_size(45)
        .y_label_area_size(80)
        .build_cartesian_2d(*dates.first().unwrap()..*dates.last().unwrap(), min_p..max_p)?;

    chart
        .configure_mesh()
        .x_label_style(("sans-serif", 11).into_font().color(&RGBColor(150, 150, 165)))
        .y_label_style(("sans-serif", 11).into_font().color(&RGBColor(150, 150, 165)))
        .axis_style(RGBColor(55, 55, 70))
        .light_line_style(RGBColor(30, 30, 45))
        .y_label_formatter(&|v| format!("{v:.0} €"))
        .draw()?;

    // Kurs
    chart
        .draw_series(LineSeries::new(
            dates.iter().zip(prices.iter()).map(|(&d, &p)| (d, p)),
            RGBColor(80, 145, 255).stroke_width(2),
        ))?
        .label("Kurs")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], RGBColor(80, 145, 255)));

    // Short SMA
    chart
        .draw_series(LineSeries::new(
            short_sma,
            RGBColor(255, 175, 50).stroke_width(1),
        ))?
        .label(format!("SMA {}", cfg.strategy.short_period))
        .legend(|(x, y)| {
            PathElement::new(vec![(x, y), (x + 20, y)], RGBColor(255, 175, 50))
        });

    // Long SMA
    chart
        .draw_series(LineSeries::new(
            long_sma,
            RGBColor(255, 75, 75).stroke_width(1),
        ))?
        .label(format!("SMA {}", cfg.strategy.long_period))
        .legend(|(x, y)| {
            PathElement::new(vec![(x, y), (x + 20, y)], RGBColor(255, 75, 75))
        });

    // BUY-Marker: grüne Dreiecke unter dem Kurs
    chart
        .draw_series(buys.iter().map(|&(d, p)| {
            TriangleMarker::new((d, p * 0.980), 12, ShapeStyle::from(&GREEN).filled())
        }))?
        .label("BUY")
        .legend(|(x, y)| {
            TriangleMarker::new((x + 10, y), 8, ShapeStyle::from(&GREEN).filled())
        });

    // SELL-Marker: rote Kreise über dem Kurs
    chart
        .draw_series(sells.iter().map(|&(d, p)| {
            Circle::new((d, p * 1.020), 7, ShapeStyle::from(&RED).filled())
        }))?
        .label("SELL")
        .legend(|(x, y)| Circle::new((x + 10, y), 7, ShapeStyle::from(&RED).filled()));

    chart
        .configure_series_labels()
        .background_style(RGBColor(28, 28, 42))
        .border_style(RGBColor(65, 65, 85))
        .label_font(("sans-serif", 13).into_font().color(&RGBColor(210, 210, 220)))
        .position(SeriesLabelPosition::UpperLeft)
        .draw()?;

    root.present()?;

    println!("Chart gespeichert → {out}");

    // Auf macOS direkt in Preview öffnen
    let _ = std::process::Command::new("open").arg(&out).spawn();

    Ok(())
}

fn sma_series(candles: &[Candle], period: usize) -> Vec<(NaiveDate, f64)> {
    if candles.len() < period {
        return vec![];
    }
    candles
        .windows(period)
        .enumerate()
        .map(|(i, w)| {
            let avg = w.iter().map(|c| c.close as f64).sum::<f64>() / period as f64 / 100.0;
            (candles[i + period - 1].timestamp.date_naive(), avg)
        })
        .collect()
}

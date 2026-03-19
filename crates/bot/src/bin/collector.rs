//! Data Collector — fetcht Marktdaten von Yahoo Finance und speichert sie in SQLite.
//!
//! Kein Paper Trading, keine Signalberechnung, keine Strategie-Logik.
//!
//! # Modi
//! - `--mode full`        : Erstabzug aller konfigurierten Intervalle (ignoriert last_timestamp)
//! - `--mode incremental` : (default) Nur neue Candles seit letztem bekannten Timestamp
//! - `--mode eod`         : Nur EOD-Intervalle (1d, 5d, 1wk, 1mo, 3mo)
//! - `--mode intraday`    : Nur Intraday-Intervalle, nur wenn US-Markt offen
//!
//! # Asset-Filter
//! - `--asset AAPL`       : nur dieses Asset
//! - (ohne Flag)          : alle Assets aus der Watchlist

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use bot::{config, db, market_data};
use chrono::{Datelike, Timelike, Utc, Weekday};

// ── Marktzeiten-Hilfsfunktionen ────────────────────────────────────────────────

/// Prüft ob der US-Aktienmarkt gerade geöffnet ist.
/// US-Märkte: Mo–Fr, 14:30–21:00 UTC
fn us_market_is_open() -> bool {
    let now = Utc::now();
    let weekday = now.weekday();
    if weekday == Weekday::Sat || weekday == Weekday::Sun {
        return false;
    }
    let hour = now.hour();
    let minute = now.minute();
    let minutes_since_midnight = hour * 60 + minute;
    // 14:30 = 870 Minuten, 21:00 = 1260 Minuten
    minutes_since_midnight >= 870 && minutes_since_midnight < 1260
}

/// Gibt zurück ob ein Intervall als Intraday gilt.
fn is_intraday(interval: &str) -> bool {
    matches!(
        interval,
        "1m" | "2m" | "5m" | "15m" | "30m" | "60m" | "90m" | "1h"
    )
}

/// Gibt zurück ob ein Intervall als EOD (End-of-Day / längerfristig) gilt.
fn is_eod(interval: &str) -> bool {
    matches!(interval, "1d" | "5d" | "1wk" | "1mo" | "3mo")
}

// ── CLI-Argumente ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum CollectorMode {
    Full,
    Incremental,
    Eod,
    Intraday,
}

impl CollectorMode {
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "full"        => Ok(Self::Full),
            "incremental" => Ok(Self::Incremental),
            "eod"         => Ok(Self::Eod),
            "intraday"    => Ok(Self::Intraday),
            other => anyhow::bail!(
                "Unbekannter Modus '{}'. Erlaubt: full, incremental, eod, intraday",
                other
            ),
        }
    }
}

#[derive(Debug)]
struct Args {
    mode:  CollectorMode,
    asset: Option<String>,
    help:  bool,
}

fn parse_args() -> Result<Args> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut mode  = CollectorMode::Incremental;
    let mut asset = None;
    let mut help  = false;
    let mut i = 0;

    while i < raw.len() {
        match raw[i].as_str() {
            "--help" | "-h" => {
                help = true;
            }
            "--mode" => {
                i += 1;
                let val = raw.get(i).ok_or_else(|| {
                    anyhow::anyhow!("--mode benötigt einen Wert")
                })?;
                mode = CollectorMode::from_str(val)?;
            }
            "--asset" => {
                i += 1;
                let val = raw.get(i).ok_or_else(|| {
                    anyhow::anyhow!("--asset benötigt einen Wert")
                })?;
                asset = Some(val.clone());
            }
            other => anyhow::bail!("Unbekanntes Argument: '{}'", other),
        }
        i += 1;
    }

    Ok(Args { mode, asset, help })
}

fn print_help() {
    println!("collector — TradingBot Datensammler");
    println!();
    println!("VERWENDUNG:");
    println!("    collector [OPTIONEN]");
    println!();
    println!("OPTIONEN:");
    println!("    --mode <MODUS>    Betriebsmodus (default: incremental)");
    println!("                        full        Erstabzug aller Intervalle");
    println!("                        incremental Nur neue Candles seit letztem Timestamp");
    println!("                        eod         Nur EOD-Intervalle (1d, 5d, 1wk, 1mo, 3mo)");
    println!("                        intraday    Nur Intraday-Intervalle (nur bei offenem Markt)");
    println!("    --asset <SYMBOL>  Nur dieses Asset verarbeiten (z.B. AAPL)");
    println!("    --help, -h        Diese Hilfe anzeigen");
    println!();
    println!("BEISPIELE:");
    println!("    collector                          # Inkrementelles Update aller Assets");
    println!("    collector --mode eod               # EOD-Update nach Marktschluss");
    println!("    collector --mode intraday          # Intraday-Update bei offenem Markt");
    println!("    collector --mode full --asset AAPL # Erstabzug nur für AAPL");
}

// ── Kollektor-Logik ────────────────────────────────────────────────────────────

/// Ergebnis eines einzelnen Fetch-Vorgangs.
#[derive(Debug)]
struct FetchResult {
    new_candles: usize,
    skipped:     bool, // true = wegen geschlossenem Markt übersprungen
}

/// Fetcht und speichert Daten für ein Asset + Intervall.
async fn collect_one(
    http:     &reqwest::Client,
    db:       &db::Db,
    asset:    &str,
    interval: &str,
    mode:     &CollectorMode,
    range:    &str,
) -> FetchResult {
    // Intraday-Intervalle bei geschlossenem Markt überspringen
    // (außer im Full-Modus — da will man die History unabhängig von Marktzeiten)
    if *mode != CollectorMode::Full && is_intraday(interval) && !us_market_is_open() {
        log::debug!("{asset}/{interval}: Markt geschlossen, überspringe");
        return FetchResult { new_candles: 0, skipped: true };
    }

    let candles = match mode {
        CollectorMode::Full => {
            // Erzwungener Erstabzug, ignoriert last_timestamp
            log::info!("{asset}/{interval}: Erstabzug erzwungen (--mode full, range={range})");
            market_data::fetch_history(http, asset, interval, range).await
        }
        CollectorMode::Incremental | CollectorMode::Eod | CollectorMode::Intraday => {
            match db.get_last_timestamp(asset, interval) {
                Ok(Some(last_ts)) => {
                    log::info!("{asset}/{interval}: inkrementelles Update seit {last_ts}");
                    market_data::fetch_since(http, asset, interval, last_ts).await
                }
                Ok(None) => {
                    log::info!("{asset}/{interval}: kein Timestamp — Erstabzug (range={range})");
                    market_data::fetch_history(http, asset, interval, range).await
                }
                Err(e) => {
                    log::warn!("{asset}/{interval}: DB-Fehler beim Lesen von last_timestamp — {e}");
                    return FetchResult { new_candles: 0, skipped: false };
                }
            }
        }
    };

    match candles {
        Ok(data) => {
            match db.upsert_candles(asset, &data, interval) {
                Ok(n) => {
                    log::info!("{asset}/{interval}: {n} neue Candles gespeichert");
                    FetchResult { new_candles: n, skipped: false }
                }
                Err(e) => {
                    log::warn!("{asset}/{interval}: DB-Fehler beim Speichern — {e}");
                    FetchResult { new_candles: 0, skipped: false }
                }
            }
        }
        Err(e) => {
            log::warn!("{asset}/{interval}: Fetch fehlgeschlagen — {e}");
            FetchResult { new_candles: 0, skipped: false }
        }
    }
}

// ── Einstiegspunkt ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = parse_args()?;

    if args.help {
        print_help();
        return Ok(());
    }

    let start = Instant::now();

    let cfg = config::Config::load(Path::new("config.toml"))?;

    // Asset-Liste: entweder explizit per --asset oder die gesamte Watchlist
    let assets: Vec<String> = match &args.asset {
        Some(a) => {
            if !cfg.assets.watchlist.contains(a) {
                log::warn!("Asset '{a}' nicht in der Watchlist — fahre trotzdem fort");
            }
            vec![a.clone()]
        }
        None => cfg.assets.watchlist.clone(),
    };

    // Intervall-Filter je nach Modus
    let intervals: Vec<String> = cfg
        .data
        .intervals
        .iter()
        .filter(|iv| match args.mode {
            CollectorMode::Eod      => is_eod(iv),
            CollectorMode::Intraday => is_intraday(iv),
            _                       => true, // Full + Incremental: alle
        })
        .cloned()
        .collect();

    if intervals.is_empty() {
        log::warn!(
            "Keine passenden Intervalle für Modus '{:?}' in der Konfiguration gefunden.",
            args.mode
        );
        return Ok(());
    }

    log::info!(
        "Collector gestartet — Modus: {:?}, Assets: {}, Intervalle: {}",
        args.mode,
        assets.len(),
        intervals.len()
    );

    if args.mode == CollectorMode::Intraday && !us_market_is_open() {
        log::warn!("Modus 'intraday' aber US-Markt ist geschlossen — alle Intraday-Intervalle werden übersprungen.");
    }

    let database = db::Db::open(&cfg.db.path)?;

    let http = reqwest::Client::new();
    let mut total_new     = 0usize;
    let mut total_skipped = 0usize;

    for asset in &assets {
        log::info!("─── {asset} ─────────────────────────────────────");
        if let Err(e) = database.ensure_asset_table(asset) {
            log::warn!("{asset}: Tabelle konnte nicht erstellt werden — {e}");
            continue;
        }

        for interval in &intervals {
            let result = collect_one(
                &http,
                &database,
                asset,
                interval,
                &args.mode,
                &cfg.data.range,
            )
            .await;

            total_new     += result.new_candles;
            if result.skipped {
                total_skipped += 1;
            }
        }
    }

    let elapsed = start.elapsed().as_secs_f64();

    // ── Zusammenfassung ────────────────────────────────────────────────────────
    println!();
    println!("══ Data Collector Zusammenfassung ════════════");
    println!("Assets:        {}", assets.len());
    println!("Intervalle:    {}", intervals.len());
    println!("Neu:           {} Candles", total_new);
    println!("Übersprungen:  {} (Markt geschlossen)", total_skipped);
    println!("Dauer:         {:.1}s", elapsed);
    println!("═════════════════════════════════════════════");

    Ok(())
}

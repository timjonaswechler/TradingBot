use anyhow::Result;
use chrono::{Datelike, Timelike, Utc, Weekday};

use crate::{config::Config, db::Db, market_data};

/// Prüft ob der US-Aktienmarkt gerade geöffnet ist (Mo–Fr, 14:30–21:00 UTC).
pub fn us_market_is_open() -> bool {
    let now = Utc::now();
    if matches!(now.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    let mins = now.hour() * 60 + now.minute();
    mins >= 870 && mins < 1260 // 14:30–21:00
}

pub fn is_intraday(interval: &str) -> bool {
    matches!(interval, "1m" | "2m" | "5m" | "15m" | "30m" | "60m" | "90m" | "1h")
}

/// Sammelt inkrementell neue Candles für alle Assets und Intervalle aus der Config.
/// Intraday-Intervalle werden übersprungen wenn der Markt geschlossen ist.
/// Gibt die Anzahl neu gespeicherter Candles zurück.
pub async fn run(cfg: &Config, db: &Db, http: &reqwest::Client) -> Result<usize> {
    let market_open = us_market_is_open();
    let mut total_new = 0usize;

    for asset in &cfg.assets.watchlist {
        db.ensure_asset_table(asset)?;

        for interval in &cfg.data.intervals {
            let last_ts = db.get_last_timestamp(asset, interval)?;

            // Intraday bei geschlossenem Markt nur überspringen wenn Daten
            // bereits vorhanden sind (inkrementelles Update sinnlos).
            // Beim Erstabzug (last_ts == None) immer laden — historische Daten
            // existieren unabhängig vom aktuellen Marktstatus.
            if is_intraday(interval) && !market_open && last_ts.is_some() {
                log::debug!("{asset}/{interval}: Markt geschlossen, überspringe inkrementelles Update");
                continue;
            }

            let n = match last_ts {
                Some(ts) => {
                    log::info!("{asset}/{interval}: inkrementelles Update seit {ts}");
                    match market_data::fetch_since(http, asset, interval, ts).await {
                        Ok(candles) => db.upsert_candles(asset, &candles, interval)?,
                        Err(e) => { log::warn!("{asset}/{interval}: fetch fehlgeschlagen – {e}"); 0 }
                    }
                }
                None => {
                    log::info!("{asset}/{interval}: Erstabzug");
                    match market_data::fetch_history(http, asset, interval, &cfg.data.range).await {
                        Ok(candles) => db.upsert_candles(asset, &candles, interval)?,
                        Err(e) => { log::warn!("{asset}/{interval}: Erstabzug fehlgeschlagen – {e}"); 0 }
                    }
                }
            };

            log::info!("{asset}/{interval}: {n} neue Candles");
            total_new += n;
        }
    }

    Ok(total_new)
}

//! Currency conversion via fend-core's `ExchangeRateFnV2` hook. fend already
//! ships every ISO 4217 code wired to a `$CURRENCY` sentinel -- registering
//! this handler is the entire integration, no custom units needed.
//!
//! Rates come from Frankfurter v2 (api.frankfurter.dev/v2/rates, no API
//! key), cached to disk so the app works offline with the last known
//! rates: 205 currencies blended from 98 central banks and official
//! sources, no request quota, no required attribution.
//
// Crypto (BTC/ETH/...) is out of scope here -- Frankfurter is fiat-only.
// A CoinGecko-backed second source would be the way to add it.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Everything Soos saves -- the document, its settings and the
/// exchange-rate cache -- lives in `data/` next to the running executable,
/// so the whole app is one folder: copy it, move it, delete it, nothing
/// touches the OS's per-user locations.
///
/// macOS is the one exception: there `data/` would sit inside `Soos.app`,
/// and dragging a new version over the old one (how a `.app` is normally
/// updated) replaces the whole bundle and takes the document with it. So on
/// macOS this uses Application Support instead, like a normal Mac app.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/Soos");
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("data")))
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// The exchange-rate cache's on-disk path, inside `data_dir()` -- shared by
/// soos-app and soos-cli so they read/write the same cache instead of each
/// keeping its own copy of this lookup.
pub fn default_cache_path() -> PathBuf {
    data_dir().join("rates.json")
}

const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
/// How long to wait before retrying after a failed background refresh --
/// without this, an offline app would hammer the network on every frame.
const RETRY_BACKOFF: Duration = Duration::from_secs(60);
const RATES_URL: &str = "https://api.frankfurter.dev/v2/rates?base=EUR";
/// Bump whenever `RATES_URL`'s source or currency coverage changes
/// incompatibly. Otherwise a cache fetched under the old source is only
/// hours old, not stale by age, and a currency missing from *that* source
/// stays broken for up to `STALE_AFTER` even after the code fix ships. A
/// cache with no `source_version` field at all deserializes it as 0 via
/// `#[serde(default)]`, which never matches `CACHE_VERSION` -- so it's
/// always treated as stale too.
const CACHE_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Clone, Default)]
struct RateCache {
    /// EUR -> currency multipliers, e.g. rates["USD"] = 1.08 means 1 EUR = 1.08 USD.
    rates: HashMap<String, f64>,
    fetched_at_unix: u64,
    #[serde(default)]
    source_version: u32,
}

/// Seconds since the Unix epoch, clamped to 0 if the clock is somehow before
/// it -- used both to stamp a freshly-fetched cache and to age-check it.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl RateCache {
    fn is_stale(&self) -> bool {
        if self.source_version != CACHE_VERSION {
            return true;
        }
        let age = now_unix().saturating_sub(self.fetched_at_unix);
        age > STALE_AFTER.as_secs()
    }
}

/// Frankfurter v2 returns a flat array of per-pair records -- one record
/// per quote currency, including an identity record for the base itself
/// (`EUR/EUR = 1.0`).
#[derive(Deserialize)]
struct RateRecord {
    quote: String,
    rate: f64,
}

/// Shared, cloneable exchange-rate source. Cloning is cheap -- clones share
/// the same in-memory cache, disk path, and in-flight/backoff state.
#[derive(Clone)]
pub struct RateSource {
    cache: Arc<RwLock<RateCache>>,
    cache_path: PathBuf,
    /// When the last refresh attempt (successful or not) started -- gates
    /// both re-entrancy (don't start a second refresh while one's running)
    /// and retry pacing (don't retry more than once per `RETRY_BACKOFF`).
    /// A single `Instant` can't distinguish "still running" from "finished
    /// a while ago" without also tracking completion, so a fetch hung past
    /// `RETRY_BACKOFF` could start one redundant concurrent thread; a
    /// dedicated in-flight flag would close that gap if it matters.
    last_attempt: Arc<Mutex<Option<Instant>>>,
}

impl RateSource {
    /// `cache_path` is a JSON file under the app's data dir -- callers
    /// (soos-app, soos-cli) resolve that with `default_cache_path` above.
    pub fn new(cache_path: PathBuf) -> Self {
        let cache = fs::read_to_string(&cache_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            cache: Arc::new(RwLock::new(cache)),
            cache_path,
            last_attempt: Arc::new(Mutex::new(None)),
        }
    }

    fn save(&self, cache: &RateCache) {
        if self.cache_path.as_os_str().is_empty() {
            return;
        }
        if let Some(parent) = self.cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(cache) {
            let _ = fs::write(&self.cache_path, json);
        }
    }

    fn refresh_blocking(&self) -> Result<(), String> {
        let records: Vec<RateRecord> = ureq::get(RATES_URL)
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .read_json()
            .map_err(|e| e.to_string())?;
        let rates = records.into_iter().map(|r| (r.quote, r.rate)).collect();
        let updated = RateCache {
            rates,
            fetched_at_unix: now_unix(),
            source_version: CACHE_VERSION,
        };
        self.save(&updated);
        *self.cache.write().unwrap() = updated;
        Ok(())
    }

    /// Blocking refresh, but only if the cache is actually stale -- what a
    /// one-shot CLI invocation wants: block once at startup rather than on
    /// every evaluation.
    pub fn refresh_if_stale_blocking(&self) {
        if self.cache.read().unwrap().is_stale() {
            let _ = self.refresh_blocking();
        }
    }

    /// Kick off a refresh on a background thread if the cache is stale, no
    /// refresh is already running, and the last attempt wasn't too recent.
    /// Cheap to call every frame -- it's a no-op almost always. `on_done`
    /// runs on the background thread once the attempt finishes (success or
    /// failure); the GUI uses it to wake the event loop and re-render.
    pub fn refresh_in_background(&self, on_done: impl FnOnce() + Send + 'static) {
        if !self.cache.read().unwrap().is_stale() {
            return;
        }
        {
            let mut last = self.last_attempt.lock().unwrap();
            if !last.is_none_or(|t| t.elapsed() >= RETRY_BACKOFF) {
                return; // already running, or retried too recently
            }
            *last = Some(Instant::now());
        }
        let this = self.clone();
        std::thread::spawn(move || {
            let _ = this.refresh_blocking();
            on_done();
        });
    }

    /// True once at least one rate is cached (in memory or from disk) --
    /// used to decide whether to surface "rates unavailable, offline?" in the UI.
    pub fn has_any_rate(&self) -> bool {
        !self.cache.read().unwrap().rates.is_empty()
    }
}

impl fend_core::ExchangeRateFnV2 for RateSource {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync + 'static>> {
        // Never fetches: a network call here would block whatever thread is
        // evaluating (the UI thread for soos-app), on every keystroke that
        // mentions a currency. Refreshing is the caller's job -- see
        // `refresh_in_background` (soos-app, every frame) and
        // `refresh_if_stale_blocking` (soos-cli, once at startup).
        //
        // Verified against fend-core's own units.rs: it defines
        // `1 <currency> = (1 / relative_to_base_currency(currency)) BASE`,
        // i.e. this must return "how many `currency` equal one base unit",
        // which for a EUR base is exactly Frankfurter's EUR-quoted rate --
        // no inversion needed, our cache already is that table.
        let cache = self.cache.read().unwrap();
        cache
            .rates
            .get(currency)
            .copied()
            .ok_or_else(|| format!("no exchange rate cached for {currency}").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a RateSource with a hand-seeded cache -- avoids a live network
    /// call in tests, and `ExchangeRateFnV2Options` has no public
    /// constructor so the trait can only realistically be exercised through
    /// `fend_core::evaluate`, not called directly.
    fn seeded(rates: &[(&str, f64)]) -> RateSource {
        let src = RateSource::new(PathBuf::new());
        let mut cache = src.cache.write().unwrap();
        cache.rates = rates.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        cache.fetched_at_unix = now_unix();
        drop(cache);
        src
    }

    #[test]
    fn missing_rate_errors_via_evaluate() {
        let mut ctx = fend_core::Context::new();
        ctx.set_exchange_rate_handler_v2(seeded(&[("EUR", 1.0)]));
        let result = fend_core::evaluate("1 USD to EUR", &mut ctx);
        assert!(result.is_err());
    }

    #[test]
    fn known_rate_converts_via_evaluate() {
        let mut ctx = fend_core::Context::new();
        // 1 EUR = 2 USD, so 1 USD = 0.5 EUR.
        ctx.set_exchange_rate_handler_v2(seeded(&[("EUR", 1.0), ("USD", 2.0)]));
        let result = fend_core::evaluate("1 USD to EUR", &mut ctx).unwrap();
        assert_eq!(result.get_main_result(), "0.5 EUR");
    }

    #[test]
    fn has_any_rate_reflects_cache_state() {
        assert!(!RateSource::new(PathBuf::new()).has_any_rate());
        assert!(seeded(&[("EUR", 1.0)]).has_any_rate());
    }

    /// End-to-end guard for the one thing a display-formatting cleanup could
    /// silently break: the value fed back into `sum` must stay raw and
    /// fend-parseable -- no `$` symbol, no `\u{2248}` -- with the symbol and
    /// rounding applied only on top, at display time.
    #[test]
    fn currency_sum_keeps_symbols_out_of_the_math() {
        let rates = seeded(&[("EUR", 1.0), ("USD", 2.0)]);
        let (_, results) = crate::recalc_document("10 USD\n5 USD\nsum", &[], &rates);
        assert_eq!(results[2], crate::LineResult::Value("15 USD".to_string()));
        assert_eq!(crate::format::format_currency("15 USD", false), "$15.00");
        assert_eq!(crate::format::format_currency("15 USD", true), "$15");
    }
}

//! Currency conversion through fend-core's `ExchangeRateFnV2` hook. fend
//! already knows every ISO 4217 code; it asks this handler for the rate.
//!
//! Rates come from Frankfurter (no API key) and are cached on disk, so
//! conversion keeps working offline on the last rates fetched. Crypto isn't
//! covered: Frankfurter only has fiat currencies.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

thread_local! {
    /// The one currency the document being recalculated on this thread may
    /// use while no rates have been downloaded -- see [`start_document`].
    static OFFLINE_CURRENCY: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Start a recalculation on this thread. Until the first rates arrive,
/// a document may use one currency: fend converts `$8 * 3` to its base
/// currency and back, so the rate cancels out and the answer is exact.
/// A second currency still needs rates -- rather than only the line that
/// converts, every value in it would carry a made-up rate.
pub(crate) fn start_document() {
    OFFLINE_CURRENCY.with_borrow_mut(|currency| *currency = None);
}

/// Whether `currency` is the document's one offline currency, claiming it
/// if the document has none yet.
fn offline_currency_is(currency: &str) -> bool {
    OFFLINE_CURRENCY.with_borrow_mut(|only| match only {
        Some(only) => only == currency,
        None => {
            *only = Some(currency.to_string());
            true
        }
    })
}

/// The rate cache inside [`crate::storage::data_dir`], shared by soos-app
/// and soos-cli.
pub fn default_cache_path() -> PathBuf {
    crate::storage::data_dir().join("rates.json")
}

/// Frankfurter's sources publish at most daily.
const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
/// The app retries a failed refresh after this long.
const RETRY_BACKOFF: Duration = Duration::from_secs(60);
/// soos-cli runs once per launcher keystroke, so after a failed refresh it
/// answers from the cache for this long instead of waiting on the network
/// each time.
const CLI_RETRY_AFTER_FAILURE: Duration = Duration::from_secs(10 * 60);
/// ureq has no timeout by default.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// Every currency quoted against EUR, in one request.
const RATES_URL: &str = "https://api.frankfurter.dev/v2/rates?base=EUR";
/// Bump when `RATES_URL`'s source or coverage changes, so a cache from the
/// old source counts as stale however recent it is. A cache without the
/// field reads as 0.
const CACHE_VERSION: u32 = 2;
/// Cap on a downloaded response (rates, and the app's update check): real
/// responses are a few KB, and ureq's own limit is unlimited.
pub const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Default)]
struct RateCache {
    /// Units of each currency per 1 EUR.
    rates: HashMap<String, f64>,
    fetched_at_unix: u64,
    #[serde(default)]
    source_version: u32,
    /// When the last refresh failed, if after the last success.
    #[serde(default)]
    failed_at_unix: u64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl RateCache {
    fn is_stale(&self) -> bool {
        self.source_version != CACHE_VERSION
            || now_unix().saturating_sub(self.fetched_at_unix) > STALE_AFTER.as_secs()
    }
}

/// One Frankfurter v2 record; the response is an array of them.
#[derive(Deserialize)]
struct RateRecord {
    quote: String,
    rate: f64,
}

/// The rates fend can use: positive and finite, since it divides by them.
/// `None` if none are left, so a broken response counts as a failed refresh
/// rather than fresh rates for [`STALE_AFTER`].
fn usable_rates(records: Vec<RateRecord>) -> Option<HashMap<String, f64>> {
    let rates: HashMap<String, f64> = records
        .into_iter()
        .filter(|r| r.rate.is_finite() && r.rate > 0.0)
        .map(|r| (r.quote, r.rate))
        .collect();
    (!rates.is_empty()).then_some(rates)
}

#[derive(Default)]
struct RefreshState {
    running: bool,
    last_attempt: Option<Instant>,
}

/// Exchange rates for fend. Clones share the cache and refresh state.
#[derive(Clone)]
pub struct RateSource {
    cache: Arc<RwLock<RateCache>>,
    cache_path: PathBuf,
    refresh: Arc<Mutex<RefreshState>>,
}

impl RateSource {
    /// Loads the cache at `cache_path`. An empty path means memory only.
    pub fn new(cache_path: PathBuf) -> Self {
        let cache = fs::read_to_string(&cache_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            cache: Arc::new(RwLock::new(cache)),
            cache_path,
            refresh: Arc::new(Mutex::new(RefreshState::default())),
        }
    }

    /// Rates held in memory only, as if just downloaded, so a test needs no
    /// file: a cache file in the temp directory is shared by every test that
    /// writes one, and they race.
    pub fn with_rates(rates: &[(&str, f64)]) -> Self {
        let source = Self::new(PathBuf::new());
        *source.cache.write().unwrap() = RateCache {
            rates: rates
                .iter()
                .map(|(code, rate)| (code.to_string(), *rate))
                .collect(),
            fetched_at_unix: now_unix(),
            source_version: CACHE_VERSION,
            failed_at_unix: 0,
        };
        source
    }

    /// Best effort: if the cache can't be written, the next run fetches again.
    fn save(&self, cache: &RateCache) {
        if self.cache_path.as_os_str().is_empty() {
            return;
        }
        let Ok(json) = serde_json::to_string(cache) else {
            return;
        };
        let _ = crate::storage::write_atomically(&self.cache_path, json.as_bytes());
    }

    fn refresh_blocking(&self) {
        let rates = ureq::get(RATES_URL)
            .config()
            .timeout_global(Some(FETCH_TIMEOUT))
            .build()
            .call()
            .ok()
            .and_then(|mut response| {
                response
                    .body_mut()
                    .with_config()
                    .limit(MAX_RESPONSE_BYTES)
                    .read_json::<Vec<RateRecord>>()
                    .ok()
            })
            .and_then(usable_rates);
        // Saved after the lock is released: the write syncs to disk, and the
        // app's UI thread reads the cache every frame.
        let cache = {
            let mut cache = self.cache.write().unwrap();
            match rates {
                Some(rates) => {
                    *cache = RateCache {
                        rates,
                        fetched_at_unix: now_unix(),
                        source_version: CACHE_VERSION,
                        failed_at_unix: 0,
                    };
                }
                None => cache.failed_at_unix = now_unix(),
            }
            cache.clone()
        };
        self.save(&cache);
    }

    /// For soos-cli: refresh before answering if the cache is stale, unless
    /// a refresh failed in the last [`CLI_RETRY_AFTER_FAILURE`].
    pub fn refresh_if_stale_blocking(&self) {
        if self.cli_should_refresh() {
            self.refresh_blocking();
        }
    }

    fn cli_should_refresh(&self) -> bool {
        let cache = self.cache.read().unwrap();
        let since_failure = now_unix().saturating_sub(cache.failed_at_unix);
        let failed_recently =
            cache.failed_at_unix != 0 && since_failure < CLI_RETRY_AFTER_FAILURE.as_secs();
        cache.is_stale() && !failed_recently
    }

    /// For the app, called every frame: refresh on a background thread if
    /// the cache is stale, nothing is running and the last attempt wasn't in
    /// the last [`RETRY_BACKOFF`]. `on_done` runs on that thread afterwards.
    pub fn refresh_in_background(&self, on_done: impl FnOnce() + Send + 'static) {
        if !self.cache.read().unwrap().is_stale() || !self.begin_refresh() {
            return;
        }
        let this = self.clone();
        std::thread::spawn(move || {
            this.refresh_blocking();
            this.refresh.lock().unwrap().running = false;
            on_done();
        });
    }

    fn begin_refresh(&self) -> bool {
        let mut state = self.refresh.lock().unwrap();
        let too_soon = state
            .last_attempt
            .is_some_and(|t| t.elapsed() < RETRY_BACKOFF);
        if state.running || too_soon {
            return false;
        }
        state.running = true;
        state.last_attempt = Some(Instant::now());
        true
    }

    /// How old the cached rates are, or `None` if there are none.
    pub fn age(&self) -> Option<Duration> {
        let cache = self.cache.read().unwrap();
        (!cache.rates.is_empty())
            .then(|| Duration::from_secs(now_unix().saturating_sub(cache.fetched_at_unix)))
    }
}

impl fend_core::ExchangeRateFnV2 for RateSource {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync + 'static>> {
        // Never fetches: evaluation would wait on the network. fend wants
        // units of `currency` per base unit, which is Frankfurter's
        // EUR-quoted rate as is.
        let cache = self.cache.read().unwrap();
        if cache.rates.is_empty() {
            if offline_currency_is(currency) {
                return Ok(1.0);
            }
            return Err("no exchange rates downloaded yet".into());
        }
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

    /// `ExchangeRateFnV2Options` has no public constructor, so the handler is
    /// tested through `fend_core::evaluate`.
    #[test]
    fn missing_rate_errors_via_evaluate() {
        let mut ctx = fend_core::Context::new();
        ctx.set_exchange_rate_handler_v2(RateSource::with_rates(&[("EUR", 1.0)]));
        let result = fend_core::evaluate("1 USD to EUR", &mut ctx);
        assert!(result.is_err());
    }

    #[test]
    fn known_rate_converts_via_evaluate() {
        let mut ctx = fend_core::Context::new();
        // 1 EUR = 2 USD, so 1 USD = 0.5 EUR.
        ctx.set_exchange_rate_handler_v2(RateSource::with_rates(&[("EUR", 1.0), ("USD", 2.0)]));
        let result = fend_core::evaluate("1 USD to EUR", &mut ctx).unwrap();
        assert_eq!(result.get_main_result(), "0.5 EUR");
    }

    #[test]
    fn unusable_rates_are_dropped_and_none_left_is_a_failure() {
        let record = |quote: &str, rate| RateRecord {
            quote: quote.to_string(),
            rate,
        };
        let rates = usable_rates(vec![
            record("USD", 1.1),
            record("GBP", 0.0),
            record("JPY", -2.0),
        ])
        .unwrap();
        assert_eq!(rates.len(), 1);
        assert_eq!(rates["USD"], 1.1);
        assert_eq!(usable_rates(vec![record("GBP", 0.0)]), None);
        assert_eq!(usable_rates(Vec::new()), None);
    }

    #[test]
    fn age_is_none_without_rates() {
        assert_eq!(RateSource::new(PathBuf::new()).age(), None);
        assert!(RateSource::with_rates(&[("EUR", 1.0)]).age().unwrap() < Duration::from_secs(60));
    }

    #[test]
    fn cli_skips_the_network_for_a_while_after_a_failure() {
        let src = RateSource::with_rates(&[("EUR", 1.0)]);
        src.cache.write().unwrap().fetched_at_unix = 0;
        assert!(src.cli_should_refresh());
        src.cache.write().unwrap().failed_at_unix = now_unix();
        assert!(!src.cli_should_refresh());
        src.cache.write().unwrap().failed_at_unix =
            now_unix() - CLI_RETRY_AFTER_FAILURE.as_secs() - 1;
        assert!(src.cli_should_refresh());
    }

    /// The symbol and rounding are display only; `sum` adds fend's values.
    #[test]
    fn currency_sum_keeps_symbols_out_of_the_math() {
        let rates = RateSource::with_rates(&[("EUR", 1.0), ("USD", 2.0)]);
        let (_, results) = crate::recalc_document("10 USD\n5 USD\nsum", &[], &rates);
        assert_eq!(results[2], crate::LineResult::Value("15 USD".to_string()));
        let shown = crate::format::shown(&results[2], false).unwrap();
        assert_eq!(shown.text, "$15.00");
    }

    #[test]
    fn begin_refresh_gates_on_running_and_backoff() {
        let src = RateSource::new(PathBuf::new());
        assert!(src.begin_refresh());
        assert!(!src.begin_refresh(), "second start while running");
        src.refresh.lock().unwrap().running = false;
        assert!(!src.begin_refresh(), "retry inside the backoff window");
        src.refresh.lock().unwrap().last_attempt = Some(Instant::now() - RETRY_BACKOFF);
        assert!(src.begin_refresh());
    }
}

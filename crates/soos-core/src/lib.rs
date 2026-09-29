//! The whole engine of Soos, no UI: fend-core wrapped with the document
//! model and the natural-language layer on top of it.
//!
//! How one recalculation flows (soos-app runs it on every edit, soos-cli once
//! per call):
//!
//! 1. [`recalc_document`] (or [`recalc_document_unless`], which the app
//!    uses so a newer edit can cancel it) builds a fresh fend context
//!    ([`new_context`]: currency handler, CSS units), registers the user's
//!    converters ([`document::define_converters`]), and runs
//!    [`document::recalc`] on a big-stack thread ([`on_big_stack`]).
//! 2. `document::recalc` walks the lines top to bottom.
//!    [`preprocess::classify`] strips comments, labels and quoted notes and
//!    sorts each line into blank, header, label or expression. An expression
//!    gets `prev`/`sum`/`avg` replaced with literal values, then goes to
//!    [`engine::eval_line_at`].
//! 3. `engine::eval_line_at` answers date and time-zone lines itself
//!    ([`preprocess::eval_date`]), rewrites natural-language phrasing
//!    ([`preprocess::rewrite`]), enforces the length and nesting caps, and
//!    hands the rest to fend with a timeout.
//! 4. Each line comes back as a [`LineResult`]. [`format::shown`] turns one
//!    into what the app's result column paints and soos-cli prints.

// These docs are for contributors (`cargo doc --document-private-items`),
// so public items may link to private ones.
#![allow(rustdoc::private_intra_doc_links)]

use std::time::SystemTime;

use chrono::{DateTime, Local};

pub mod currency;
mod document;
mod engine;
pub mod format;
pub mod highlight;
mod preprocess;
pub mod storage;

pub use document::{LineResult, RawConverter, MAX_CONVERTERS};

/// A recalculated document: one result per converter (its value or why
/// it's invalid), then one per line.
pub type Recalculated = (Vec<Result<String, String>>, Vec<LineResult>);

/// A fresh fend context with the currency handler and Soos's CSS units.
fn new_context(rates: &currency::RateSource) -> fend_core::Context {
    let mut ctx = fend_core::Context::new();
    ctx.set_exchange_rate_handler_v2(rates.clone());
    engine::register_builtin_units(&mut ctx);
    ctx
}

/// Runs `f` on a new thread with an 8 MiB stack. The length and nesting
/// caps in [`engine`] bound fend's recursion; this makes sure recursion up
/// to those caps fits, whichever thread calls in (a Windows GUI thread can
/// have 1 MiB). One thread per recalculation, not per line.
fn on_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    const STACK_SIZE: usize = 8 * 1024 * 1024;
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, f)
            .expect("failed to spawn recalc thread")
            .join()
            .expect("recalc thread panicked")
    })
}

/// Recalculate a whole document with currency and the user's converters.
pub fn recalc_document(
    source: &str,
    converters: &[RawConverter],
    rates: &currency::RateSource,
) -> Recalculated {
    recalc_document_at(source, converters, rates, SystemTime::now())
}

/// [`recalc_document`] as of `now`, so `today` and `now` lines can be
/// tested. Every line in one recalculation sees the same clock.
pub fn recalc_document_at(
    source: &str,
    converters: &[RawConverter],
    rates: &currency::RateSource,
    now: SystemTime,
) -> Recalculated {
    // Without a cancel, the recalculation always finishes.
    recalc_on_big_stack(source, converters, rates, now, None).unwrap_or_default()
}

/// [`recalc_document`], stopping as soon as `cancelled` returns true: the
/// app's recalculation of a document the user has since edited again.
/// `None` if it was cancelled.
pub fn recalc_document_unless(
    source: &str,
    converters: &[RawConverter],
    rates: &currency::RateSource,
    cancelled: impl Fn() -> bool + Send + Sync + 'static,
) -> Option<Recalculated> {
    let cancelled: engine::Cancel = std::sync::Arc::new(cancelled);
    recalc_on_big_stack(
        source,
        converters,
        rates,
        SystemTime::now(),
        Some(cancelled),
    )
}

fn recalc_on_big_stack(
    source: &str,
    converters: &[RawConverter],
    rates: &currency::RateSource,
    now: SystemTime,
    cancel: Option<engine::Cancel>,
) -> Option<Recalculated> {
    let now = DateTime::<Local>::from(now);
    on_big_stack(move || {
        engine::set_cancel(cancel);
        currency::start_document();
        let mut ctx = new_context(rates);
        let converter_results = document::define_converters(&mut ctx, converters);
        let line_results = document::recalc(&mut ctx, source, now);
        (!engine::cancelled()).then_some((converter_results, line_results))
    })
}

/// What the app would show for `input`, for soos-cli and the launchers:
/// `input` is recalculated like a document, and the last line that shows a
/// result is formatted by [`format::shown`]. `None` if no line shows one.
pub fn evaluate_one(
    input: &str,
    converters: &[RawConverter],
    rates: &currency::RateSource,
    high_precision: bool,
) -> Option<format::Shown> {
    let (_, results) = recalc_document(input, converters, rates);
    results
        .iter()
        .rev()
        .find_map(|result| format::shown(result, high_precision))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    /// Hostile input must neither panic (fatal under `panic = "abort"`) nor
    /// hang the thread the app's UI waits on.
    #[test]
    fn hostile_inputs_never_panic_or_hang() {
        let rates = currency::RateSource::new(PathBuf::new());
        let converters: Vec<RawConverter> = Vec::new();

        let long_line = "1+".repeat(3000);
        let dashes = format!("{}1", "-".repeat(3000));
        let power_tower = format!("{}1", "1^".repeat(2000));
        let deep_parens = "(".repeat(65);
        let huge_sum_block = format!("{}sum", "1\n".repeat(10_000));

        let hostile: Vec<String> = vec![
            "today + 999999999999999999 days".to_string(),
            "today - 99999999999 years".to_string(),
            "9999999!".to_string(),
            "10^10^10".to_string(),
            "2^2^40".to_string(),
            long_line,
            dashes,
            power_tower,
            deep_parens,
            "99:99pm in Tokyo".to_string(),
            "\"unterminated".to_string(),
            "\u{1F600}\u{200F}\u{202E}".to_string(), // emoji + RTL marks
            "1 + 1\r\n2 + 2\r\n".to_string(),
            huge_sum_block,
        ];

        for line in hostile {
            let start = Instant::now();
            let (_converter_results, results) = recalc_document(&line, &converters, &rates);
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "line took too long: {line:.60}"
            );
            assert!(!results.is_empty() || line.is_empty());

            let start = Instant::now();
            let _ = evaluate_one(&line, &converters, &rates, false);
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "evaluate_one took too long: {line:.60}"
            );
        }
    }

    /// A converter's base may contain `^`, so it gets the same guards.
    #[test]
    fn hostile_converter_base_never_hangs() {
        let rates = currency::RateSource::new(PathBuf::new());
        let converters = vec![RawConverter {
            unit: "bignum".to_string(),
            aliases: Vec::new(),
            base: "m^99999999999".to_string(),
            factor: "1".to_string(),
        }];
        let start = Instant::now();
        let (converter_results, _) = recalc_document("1 bignum", &converters, &rates);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(converter_results.len(), 1);
    }

    /// 1 EUR = 2 USD, read from a cache file (read once, on construction).
    fn fixed_rates(name: &str) -> currency::RateSource {
        let dir = std::env::temp_dir().join(format!("soos-lib-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rates.json");
        std::fs::write(
            &path,
            r#"{"rates":{"EUR":1.0,"USD":2.0},"fetched_at_unix":0,"source_version":2}"#,
        )
        .unwrap();
        let rates = currency::RateSource::new(path);
        let _ = std::fs::remove_dir_all(&dir);
        rates
    }

    #[test]
    fn evaluate_one_shows_what_the_app_shows() {
        let rates = fixed_rates("shows");
        let shown = evaluate_one("Rent: $1234.5 * 2 // yearly", &[], &rates, false).unwrap();
        assert_eq!(shown.text, "$2,469.00");
        assert_eq!(shown.copy, "$2469.00");
        assert_eq!(shown.error, None);

        let precise = evaluate_one("$1234.5 * 2", &[], &rates, true).unwrap();
        assert_eq!(precise.text, "$2,469");
    }

    /// Every styled currency, copied from the result column and pasted back
    /// in, is the same amount in the same currency.
    #[test]
    fn copied_currency_results_read_back_as_the_same_value() {
        let codes: Vec<String> = format::CURRENCY_STYLES
            .iter()
            .map(|s| format!("\"{}\":2.0", s.code))
            .collect();
        let dir = std::env::temp_dir().join(format!("soos-lib-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rates.json");
        let json = format!(
            r#"{{"rates":{{"EUR":1.0,{}}},"fetched_at_unix":0,"source_version":2}}"#,
            codes.join(",")
        );
        std::fs::write(&path, json).unwrap();
        let rates = currency::RateSource::new(path);
        let _ = std::fs::remove_dir_all(&dir);

        for style in format::CURRENCY_STYLES {
            for amount in ["1234567.891", "-42.5", "0.5"] {
                let line = format!("{amount} {}", style.code);
                let first = evaluate_one(&line, &[], &rates, false).unwrap();
                let again = evaluate_one(&first.copy, &[], &rates, false).unwrap();
                assert_eq!(again.error, None, "{line} -> {:?}", first.copy);
                assert_eq!(again.text, first.text, "{line} -> {:?}", first.copy);
            }
        }
    }

    #[test]
    fn a_conversion_with_no_rates_downloaded_says_so() {
        let rates = currency::RateSource::new(PathBuf::new());
        let shown = evaluate_one("$8 in EUR", &[], &rates, false).unwrap();
        assert_eq!(shown.text, "no rates yet");
    }

    #[test]
    fn evaluate_one_uses_converters_and_the_last_line_with_a_result() {
        let rates = currency::RateSource::new(PathBuf::new());
        let teu = RawConverter {
            unit: "teu".to_string(),
            aliases: Vec::new(),
            base: "cbm".to_string(),
            factor: "33.2".to_string(),
        };
        let shown = evaluate_one("2 teu in cbm", &[teu], &rates, false).unwrap();
        assert_eq!(shown.text, "66.4 m^3");

        let shown = evaluate_one("1\n2\nsum\n# notes", &[], &rates, false).unwrap();
        assert_eq!(shown.text, "3");
        assert_eq!(evaluate_one("# only a header", &[], &rates, false), None);
    }

    /// With no rates downloaded, one currency still works: its rate cancels
    /// out. A second one anywhere in the document is an error, since every
    /// value in it would carry a made-up rate.
    #[test]
    fn one_currency_works_before_any_rates_are_downloaded() {
        let rates = currency::RateSource::new(PathBuf::new());
        let (_, results) = recalc_document("$8 * 3\n$2\nsum", &[], &rates);
        let shown: Vec<_> = results
            .iter()
            .map(|r| format::shown(r, false).unwrap().text)
            .collect();
        assert_eq!(shown, ["$24.00", "$2.00", "$26.00"]);

        let (_, results) = recalc_document("x = $5\ny = 3 EUR\nx to EUR\nx * 2", &[], &rates);
        let shown: Vec<_> = results
            .iter()
            .map(|r| format::shown(r, false).unwrap().text)
            .collect();
        assert_eq!(shown, ["$5.00", "no rates yet", "no rates yet", "$10.00"]);
    }

    /// A cancelled recalculation stops inside a slow line, not after it.
    #[test]
    fn a_cancelled_recalculation_stops_promptly() {
        let rates = currency::RateSource::new(PathBuf::new());
        let slow = "9999999!\n".repeat(10);
        let start = Instant::now();
        let cancelled = recalc_document_unless(&slow, &[], &rates, || true);
        assert!(cancelled.is_none());
        assert!(start.elapsed() < Duration::from_millis(150));

        let (_, results) = recalc_document_unless("1 + 1", &[], &rates, || false).unwrap();
        assert_eq!(results, [LineResult::Value("2".to_string())]);
    }

    #[test]
    fn evaluate_one_errors_carry_the_short_label_and_the_full_message() {
        let rates = currency::RateSource::new(PathBuf::new());
        let shown = evaluate_one("5 metr", &[], &rates, false).unwrap();
        assert_eq!(shown.text, "unknown metr");
        assert_eq!(shown.error.as_deref(), Some("unknown identifier 'metr'"));
    }
}

//! The whole engine of Soos, no UI: fend-core wrapped with the document
//! model and the natural-language layer on top of it.
//!
//! How one recalculation flows (soos-app runs it on every edit, soos-cli once
//! per call):
//!
//! 1. [`recalc_document`] (or [`recalc_document_unless`], which the app
//!    uses so a newer edit can cancel it) builds a fresh fend context
//!    ([`new_context`]: currency handler, CSS units), registers the user's
//!    converters ([`document::define_converters`]) and then `root`, and
//!    runs [`document::recalc`] on a big-stack thread ([`STACK_SIZE`]).
//! 2. `document::recalc` walks the lines top to bottom.
//!    [`preprocess::classify`] strips comments, labels and quoted notes and
//!    sorts each line into blank, header, label or expression. An expression
//!    gets `prev`/`sum`/`avg` replaced with literal values, then goes to
//!    [`engine::eval_line_at`].
//! 3. `engine::eval_line_at` answers date and time-zone lines itself
//!    ([`preprocess::eval_date`]), rewrites natural-language phrasing
//!    ([`preprocess::rewrite`]), enforces the length and nesting caps,
//!    refuses what fend can't interrupt (`engine::guard`), and hands the
//!    rest to fend with a timeout.
//!    [`watchdog`] gives each line 2 seconds: one that runs over shows
//!    `too slow`, and the document is recalculated on a fresh thread
//!    without it.
//! 4. Each line comes back as a [`LineResult`]. [`format::shown`] turns one
//!    into what the app's result column paints and soos-cli prints; with the
//!    decimal comma on, the app swaps it with [`format::Shown::swapped`].

// These docs are for contributors (`cargo doc --document-private-items`),
// so public items may link to private ones.
#![allow(rustdoc::private_intra_doc_links)]

use std::time::SystemTime;

use chrono::{DateTime, Local};

pub mod currency;
mod document;
mod engine;
mod error;
pub mod format;
pub mod highlight;
mod preprocess;
pub mod storage;
mod watchdog;

pub use document::{LineResult, RawConverter, MAX_CONVERTERS};
pub use error::LineError;

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

/// The stack of the thread a recalculation runs on: 8 MiB. The length and
/// nesting caps in [`engine`] bound fend's recursion; this makes sure
/// recursion up to those caps fits, whichever thread calls in (a Windows GUI
/// thread can have 1 MiB). One thread per recalculation, not per line.
///
/// A debug build gets 64 MiB: its frames are far bigger. Only touched pages
/// are committed, so the larger reserve costs nothing until it is used.
const STACK_SIZE: usize = if cfg!(debug_assertions) { 64 } else { 8 } * 1024 * 1024;

/// Runs `f` on a new thread with a [`STACK_SIZE`] stack.
#[cfg(test)]
fn on_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
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
    recalc_on_big_stack(&watchdog::WATCHDOG, source, converters, rates, now, None)
        .unwrap_or_default()
}

/// [`recalc_document`], stopping as soon as `cancelled` returns true: the
/// app's recalculation of a document the user has since edited again.
/// `None` if it was cancelled. A line fend can't interrupt still holds it
/// until the watchdog gives up on that line (see [`watchdog`]).
pub fn recalc_document_unless(
    source: &str,
    converters: &[RawConverter],
    rates: &currency::RateSource,
    cancelled: impl Fn() -> bool + Send + Sync + 'static,
) -> Option<Recalculated> {
    let cancelled: engine::Cancel = std::sync::Arc::new(cancelled);
    recalc_on_big_stack(
        &watchdog::WATCHDOG,
        source,
        converters,
        rates,
        SystemTime::now(),
        Some(cancelled),
    )
}

/// The recalculation, on a [`STACK_SIZE`] thread that `watchdog` watches.
fn recalc_on_big_stack(
    watchdog: &std::sync::Arc<watchdog::Watchdog>,
    source: &str,
    converters: &[RawConverter],
    rates: &currency::RateSource,
    now: SystemTime,
    cancel: Option<engine::Cancel>,
) -> Option<Recalculated> {
    let now = DateTime::<Local>::from(now);
    let (source, converters, rates) = (source.to_owned(), converters.to_vec(), rates.clone());
    let wanted = cancel.clone();
    let cancelled = move || wanted.as_ref().is_some_and(|cancelled| cancelled());
    watchdog::run(watchdog, STACK_SIZE, &cancelled, move || {
        engine::set_cancel(cancel.clone());
        currency::start_document();
        let mut ctx = new_context(&rates);
        let converter_results = document::define_converters(&mut ctx, &converters);
        // Last, so `ans` on the first line is `root`, not a converter's check.
        engine::register_root(&mut ctx);
        let seed = document::converters_key(&converters);
        let line_results = document::recalc(&mut ctx, &source, now, seed);
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
            "99:99PM in Tokyo".to_string(),
            "\"unterminated".to_string(),
            "\u{1F600}\u{200F}\u{202E}".to_string(), // emoji + RTL marks
            // fend prints a string literal as is, so its text is a "value":
            // a leading space and a multibyte letter, then a multibyte digit
            // under `in sci`.
            "' \u{e9}'".to_string(),
            "'\u{663}' in sci".to_string(),
            // fend never checks its interrupt on these (see `engine::guard`):
            // a long shift, dice that multiply their outcomes, and a unit
            // raised to a large power.
            "4 << 100000000".to_string(),
            "0xff << 99999999999999999999".to_string(),
            "1 << (100000000 in base 3)".to_string(),
            "d1000 + d1000".to_string(),
            "cm^400000 kg".to_string(),
            // Powers that add up across a line, and `root` below 1.
            "u = cm^5000\nu^20 kg".to_string(),
            "cm^10^5 kg".to_string(),
            "root (1/100000) (cm) * kg".to_string(),
            // A percent phrase in brackets repeats its amount: chained or
            // nested ones would double the line each time.
            format!("({}1)", "1% off ".repeat(30)),
            format!("{}100{}", "(5% off ".repeat(30), ")".repeat(30)),
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

    /// The guard counts a converter at its base's power: `bigY` is `ym^6`, so
    /// `bigY^8000` is a power of 48,000, which took 22 s when it counted 1.
    #[test]
    fn a_converter_counts_at_its_base_power() {
        let rates = currency::RateSource::new(PathBuf::new());
        let converters = vec![RawConverter {
            unit: "bigY".to_string(),
            aliases: Vec::new(),
            base: "ym^6".to_string(),
            factor: "1".to_string(),
        }];
        let start = Instant::now();
        let (converter_results, results) =
            recalc_document("bigY^8000 kg\nbigY^1000", &converters, &rates);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(converter_results[0].is_ok(), "{converter_results:?}");
        assert!(
            matches!(&results[0], LineResult::Error(e) if e.short() == "too large"),
            "{:?}",
            results[0]
        );
        assert!(
            matches!(&results[1], LineResult::Value(_)),
            "{:?}",
            results[1]
        );
    }

    /// A base built on an earlier converter is counted at that converter's
    /// power while the table is read, so it can't hold up the table either.
    #[test]
    fn a_converter_built_on_another_is_counted_in_full() {
        let rates = currency::RateSource::new(PathBuf::new());
        let row = |unit: &str, base: &str| RawConverter {
            unit: unit.to_string(),
            aliases: Vec::new(),
            base: base.to_string(),
            factor: "1".to_string(),
        };
        let converters = vec![row("bigY", "ym^6"), row("stall", "bigY^8000 kg")];
        let start = Instant::now();
        let (converter_results, results) = recalc_document("1 + 1", &converters, &rates);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(converter_results[0].is_ok() && converter_results[1].is_err());
        assert_eq!(results[0], LineResult::Value("2".to_string()));
    }

    /// `ans` on the first line is `root`, a function, even with converters
    /// defined before the document.
    #[test]
    fn ans_on_the_first_line_is_not_a_converters_value() {
        let rates = currency::RateSource::new(PathBuf::new());
        let converters = vec![RawConverter {
            unit: "lap".to_string(),
            aliases: Vec::new(),
            base: "m".to_string(),
            factor: "400".to_string(),
        }];
        let (_, results) = recalc_document("ans", &converters, &rates);
        assert!(
            matches!(&results[0], LineResult::Error(e) if e.to_string().contains("not a number")),
            "{:?}",
            results[0]
        );
    }

    /// A line over the limit is given up on, and not run again until it or
    /// a line it uses changes. Every shape known to get past fend's own
    /// interrupt is refused by the guard, so this uses `9999999!`, which fend
    /// stops at 200 ms, with a lower limit: the watchdog acts first, and a
    /// debug build's slower start can't reach the half as long again in
    /// which a line is forgiven.
    #[test]
    fn a_line_that_runs_too_long_is_given_up_on() {
        let limit = Duration::from_millis(100);
        let watchdog = std::sync::Arc::new(watchdog::Watchdog::new(limit));
        let rates = currency::RateSource::new(PathBuf::new());
        let recalc = |source: &str| {
            let start = Instant::now();
            let (_, results) =
                recalc_on_big_stack(&watchdog, source, &[], &rates, SystemTime::now(), None)
                    .unwrap();
            (results, start.elapsed())
        };
        let too_complex = |result: &LineResult| {
            matches!(result, LineResult::Error(e) if e.short() == "too complex"
                && e.to_string().contains("2 seconds"))
        };
        let doc = "n = 9999999\nn!\n1 + 1";

        let (results, took) = recalc(doc);
        assert!(too_complex(&results[1]), "{:?}", results[1]);
        assert_eq!(results[2], LineResult::Value("2".to_string()));
        assert!(took < Duration::from_secs(5), "{took:?}");

        // Typing in another line doesn't run it again.
        let (results, took) = recalc(&doc.replace("1 + 1", "1 + 2"));
        assert!(too_complex(&results[1]), "{:?}", results[1]);
        assert_eq!(results[2], LineResult::Value("3".to_string()));
        assert!(took < Duration::from_secs(1), "{took:?}");

        // Changing a line it uses does.
        let (results, _) = recalc(&doc.replace("9999999", "5"));
        assert_eq!(results[1], LineResult::Value("120".to_string()));

        // So does changing a function the line reaches only through another
        // one, defined after it: fend looks `slow` up when `t3` runs.
        let watchdog = std::sync::Arc::new(watchdog::Watchdog::new(limit));
        let recalc = |source: &str| {
            recalc_on_big_stack(&watchdog, source, &[], &rates, SystemTime::now(), None)
                .unwrap()
                .1
        };
        let slow_after = "t3 = \\x. slow(x)\nslow = \\x. x!\nt3(9999999)";
        assert!(too_complex(&recalc(slow_after)[2]));
        let results = recalc(&slow_after.replace("x!", "x + 1"));
        assert_eq!(results[2], LineResult::Value("10000000".to_string()));
    }

    /// A plain number's power in a function is fend's to interrupt, however
    /// large the argument.
    #[test]
    fn a_plain_numbers_power_in_a_function_is_allowed() {
        let rates = currency::RateSource::new(PathBuf::new());
        let source = "growth = \\y. 1000 * 1.05^y\ngrowth 10\ngrowth 1000000000\n\
                      pow2 = \\x. 2^x\npow2 10\nr = 0.05\ncomp = \\y. 1000 * (1 + r)^y\ncomp 2";
        let start = Instant::now();
        let (_, results) = recalc_document(source, &[], &rates);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        let shown: Vec<_> = results
            .iter()
            .map(|r| format::shown(r, false).unwrap().text)
            .collect();
        assert_eq!(shown[1], "1,628.89462677744140625");
        assert_eq!(shown[2], "too complex");
        assert_eq!(shown[4], "1,024");
        assert_eq!(shown[7], "1,102.5");
    }

    /// fend runs an argument inside the call it is passed to, so calls
    /// inside calls add up; past what the stack holds, they're refused.
    /// On the recalculation's own thread: a test thread's stack is smaller.
    #[test]
    fn functions_called_too_deeply_are_refused() {
        let rates = currency::RateSource::new(PathBuf::new());
        let shown = |source: &str| -> Vec<String> {
            recalc_document(source, &[], &rates)
                .1
                .iter()
                .map(|r| format::shown(r, false).map_or(String::new(), |s| s.text))
                .collect()
        };
        let deep = format!("deepf = \\x. {}x{} + 1", "(".repeat(55), ")".repeat(55));
        let calls = |n: usize| format!("{}1{}", "deepf(".repeat(n), ")".repeat(n));
        let results = shown(&format!("{deep}\n{}\n{}", calls(10), calls(20)));
        assert_eq!(results[1], "11");
        assert_eq!(results[2], "too nested");

        let chain: Vec<String> = std::iter::once("f1 = \\x. x + 1".to_string())
            .chain((2..=200).map(|k| format!("f{k} = \\x. f{}(x) + 1", k - 1)))
            .chain(["f200(0)".to_string()])
            .collect();
        assert_eq!(
            shown(&chain.join("\n")).last().map(String::as_str),
            Some("200")
        );
    }

    /// 1 EUR = 2 USD.
    fn fixed_rates() -> currency::RateSource {
        currency::RateSource::with_rates(&[("EUR", 1.0), ("USD", 2.0), ("GBP", 1.0), ("CAD", 1.0)])
    }

    #[test]
    fn evaluate_one_shows_what_the_app_shows() {
        let rates = fixed_rates();
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
        let pairs: Vec<(&str, f64)> = format::CURRENCY_STYLES
            .iter()
            .map(|s| (s.code, 2.0))
            .collect();
        let rates = currency::RateSource::with_rates(&pairs);

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

    /// `$5 + 1` is `$6`, with a plain number in a variable's unit too.
    #[test]
    fn a_plain_number_added_to_money_is_money() {
        let rates = fixed_rates();
        let (_, results) = recalc_document(
            "$5 + 1\n1 + $5\n$5 * 2 + 1\nrent = $1800\nrent + 50\n$5 + 1 EUR\n$5 + 1 in EUR\n($5 + 1) in EUR",
            &[],
            &rates,
        );
        let shown: Vec<_> = results
            .iter()
            .map(|r| format::shown(r, false).unwrap().text)
            .collect();
        assert_eq!(
            shown,
            [
                "$6.00",
                "$6.00",
                "$11.00",
                "$1,800.00",
                "$1,850.00",
                "$7.00",
                "\u{20ac}3.00",
                "\u{20ac}3.00"
            ]
        );
    }

    /// A symbol after the number is the same amount as one before it, and
    /// any currency rounds, not only the ones with a symbol.
    #[test]
    fn a_symbol_after_the_number_and_a_code_without_one_round() {
        let rates = fixed_rates();
        let (_, results) = recalc_document(
            "5$\n1.234\u{20ac}\n5 \u{a3}\n12.345 CAD\n5.555 dollars",
            &[],
            &rates,
        );
        let shown: Vec<_> = results
            .iter()
            .map(|r| format::shown(r, false).unwrap().text)
            .collect();
        assert_eq!(
            shown,
            ["$5.00", "\u{20ac}1.23", "\u{a3}5.00", "12.35 CAD", "$5.56"]
        );
    }

    /// A currency code is a unit too: `EUR = 2` would make `5 EUR` 10.
    #[test]
    fn a_variable_cannot_take_a_currency_code() {
        let rates = fixed_rates();
        let (_, results) = recalc_document("EUR = 2\n5 EUR\n$5 + 1", &[], &rates);
        let shown: Vec<_> = results
            .iter()
            .map(|r| format::shown(r, false).unwrap().text)
            .collect();
        assert_eq!(shown, ["name in use", "\u{20ac}5.00", "$6.00"]);
    }

    /// The length and nesting caps are meant to keep recursion inside the
    /// recalculation thread's stack, a debug build's bigger frames included.
    /// A block this long gets through them, so its `sum` has to fit.
    #[test]
    fn a_sum_over_a_long_block_fits_the_stack() {
        let rates = fixed_rates();
        for line in ["1234567.89 USD", "1234.5", "1/3"] {
            let block = format!("{}sum", format!("{line}\n").repeat(120));
            let (_, results) = recalc_document(&block, &[], &rates);
            assert!(
                matches!(results.last(), Some(LineResult::Value(_))),
                "{line}: {:?}",
                results.last()
            );
        }
    }

    /// The block is written into the `sum` line, which has a length cap, so
    /// a very long block says so instead of "too long" on a one-word line.
    #[test]
    fn a_block_too_long_to_add_up_says_so() {
        let rates = fixed_rates();
        let block = format!("{}sum", "12\n".repeat(400));
        let (_, results) = recalc_document(&block, &[], &rates);
        let last = format::shown(results.last().unwrap(), false).unwrap();
        assert_eq!(last.text, "too many lines");
        assert!(last.error.unwrap().contains("start a new block"));
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

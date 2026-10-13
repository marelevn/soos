//! The bridge between the Android app and soos-core. Kotlin's `Engine`
//! object (`io.github.marelevn.soos.Engine`) declares the `external fun`s
//! that the `Java_...` functions below implement; each one only converts
//! its arguments and calls a plain function, so the host can test the logic
//! without a JVM.
//!
//! Everything crosses as strings, `int[]` and numbers, never as objects:
//! results go back as one JSON document ([`recalc`]), which `org.json`
//! reads on Android without another dependency.
//!
//! The rate cache path comes from Kotlin ([`init`]), so soos-core's
//! `storage::data_dir()` (the folder next to the executable) is never used.

// These docs are for contributors (`cargo doc --document-private-items`),
// so the crate's docs may link to private items.
#![allow(rustdoc::private_intra_doc_links)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jintArray, jlong, jstring, JNI_TRUE};
use jni::JNIEnv;
use serde_json::{json, Value};
use soos_core::currency::RateSource;
use soos_core::format::{result_cells, swap_separators};
use soos_core::highlight::{tokens, TokenKind};
use soos_core::{recalc_document_unless, RawConverter};

static RATES: OnceLock<RateSource> = OnceLock::new();

/// The newest recalculation Kotlin asked for: a job with an older
/// generation is cancelled, as the desktop app's `Recalculator` does.
static NEWEST: AtomicU64 = AtomicU64::new(0);

/// Memory-only rates if [`init`] hasn't run, so a missed call can't panic.
fn rates() -> &'static RateSource {
    RATES.get_or_init(|| RateSource::new(PathBuf::new()))
}

/// Loads the rate cache at `path`. Android may recreate the activity inside
/// one process, so a second call keeps the first source.
fn init(path: &str) {
    let _ = RATES.set(RateSource::new(PathBuf::from(path)));
}

/// What [`highlight`] calls each [`TokenKind`]; Kotlin's `Engine` has the
/// same numbers.
fn kind_code(kind: TokenKind) -> i32 {
    match kind {
        TokenKind::Comment => 0,
        TokenKind::Header => 1,
        TokenKind::Label => 2,
        TokenKind::Keyword => 3,
        TokenKind::ConversionWord => 4,
    }
}

/// Syntax spans for the whole `text` as `[start, end, kind, ...]`, with
/// `start` and `end` in UTF-16 units, which is what Kotlin's strings index
/// by. soos-core gives byte ranges per line, so a `€` or an emoji before a
/// token would shift every span after it if they were passed on as they are.
fn highlight(text: &str) -> Vec<i32> {
    let units = |s: &str| s.encode_utf16().count() as i32;
    let mut spans = Vec::new();
    let mut line_start = 0;
    for line in text.split('\n') {
        for (range, kind) in tokens(line) {
            let start = line_start + units(&line[..range.start]);
            spans.extend([start, start + units(&line[range]), kind_code(kind)]);
        }
        line_start += units(line) + 1;
    }
    spans
}

/// Recalculates `text` like the desktop app does, and answers with JSON:
///
/// ```text
/// {"converters": [{"ok": "..."} | {"error": "..."}],
///  "lines": [null | {"text": "...", "copy": "...", "error": "..." | null,
///                    "total": false}]}
/// ```
///
/// `converters_json` is `RawConverter`s, already trimmed. With
/// `decimal_comma`, the text and each converter's base and factor are read
/// with `.` and `,` swapped and the results are swapped back, so the
/// engine only ever sees `1,234.5`. `None` once `cancelled` says so.
fn recalc(
    text: &str,
    converters_json: &str,
    high_precision: bool,
    decimal_comma: bool,
    rates: &RateSource,
    cancelled: impl Fn() -> bool + Send + Sync + 'static,
) -> Option<String> {
    // Kotlin writes this itself, so a document that doesn't parse is a bug
    // there; the lines still calculate, without the converters.
    let mut converters: Vec<RawConverter> =
        serde_json::from_str(converters_json).unwrap_or_default();
    let engine_text = if decimal_comma {
        for converter in &mut converters {
            converter.base = swap_separators(&converter.base);
            converter.factor = swap_separators(&converter.factor);
        }
        swap_separators(text)
    } else {
        text.to_owned()
    };
    let (converter_results, results) =
        recalc_document_unless(&engine_text, &converters, rates, cancelled)?;

    let converters: Vec<Value> = converter_results
        .into_iter()
        .map(|result| match result {
            Ok(value) if decimal_comma => json!({ "ok": swap_separators(&value) }),
            Ok(value) => json!({ "ok": value }),
            Err(error) => json!({ "error": error }),
        })
        .collect();
    let lines: Vec<Value> = result_cells(text, &results, high_precision, decimal_comma)
        .into_iter()
        .map(|cell| match cell {
            None => Value::Null,
            Some(cell) => json!({
                "text": cell.shown.text,
                "copy": cell.shown.copy,
                "error": cell.shown.error,
                "total": cell.total,
            }),
        })
        .collect();
    Some(json!({ "converters": converters, "lines": lines }).to_string())
}

/// A Java string as a Rust one; `None` if the JVM has an exception pending.
fn read(env: &mut JNIEnv, string: &JString) -> Option<String> {
    env.get_string(string).ok().map(String::from)
}

#[no_mangle]
pub extern "system" fn Java_io_github_marelevn_soos_Engine_init<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    rates_path: JString<'l>,
) {
    if let Some(path) = read(&mut env, &rates_path) {
        init(&path);
    }
}

#[no_mangle]
pub extern "system" fn Java_io_github_marelevn_soos_Engine_cancelOlderThan(
    _env: JNIEnv,
    _class: JClass,
    generation: jlong,
) {
    NEWEST.store(generation as u64, Ordering::Relaxed);
}

#[no_mangle]
pub extern "system" fn Java_io_github_marelevn_soos_Engine_recalc<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    generation: jlong,
    text: JString<'l>,
    converters_json: JString<'l>,
    high_precision: jboolean,
    decimal_comma: jboolean,
) -> jstring {
    let (Some(text), Some(converters_json)) =
        (read(&mut env, &text), read(&mut env, &converters_json))
    else {
        return std::ptr::null_mut();
    };
    let generation = generation as u64;
    recalc(
        &text,
        &converters_json,
        high_precision == JNI_TRUE,
        decimal_comma == JNI_TRUE,
        rates(),
        move || NEWEST.load(Ordering::Relaxed) != generation,
    )
    .and_then(|json| env.new_string(json).ok())
    .map_or(std::ptr::null_mut(), JString::into_raw)
}

#[no_mangle]
pub extern "system" fn Java_io_github_marelevn_soos_Engine_highlight<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    text: JString<'l>,
) -> jintArray {
    let Some(text) = read(&mut env, &text) else {
        return std::ptr::null_mut();
    };
    let spans = highlight(&text);
    let Ok(array) = env.new_int_array(spans.len() as i32) else {
        return std::ptr::null_mut();
    };
    match env.set_int_array_region(&array, 0, &spans) {
        Ok(()) => array.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Blocks for the network (5 seconds at most), so Kotlin calls it off the
/// main thread.
#[no_mangle]
pub extern "system" fn Java_io_github_marelevn_soos_Engine_refreshRates(
    _env: JNIEnv,
    _class: JClass,
) {
    rates().refresh_if_stale_blocking_with_backoff();
}

/// How old the cached rates are in seconds, or -1 if there are none.
#[no_mangle]
pub extern "system" fn Java_io_github_marelevn_soos_Engine_ratesAgeSeconds(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    rates()
        .age()
        .map_or(-1, |age| age.as_secs().try_into().unwrap_or(jlong::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, converters: &str, decimal_comma: bool) -> Value {
        let rates = RateSource::with_rates(&[("EUR", 1.0), ("USD", 2.0)]);
        let json = recalc(text, converters, false, decimal_comma, &rates, || false).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    fn line_text(doc: &Value, i: usize) -> &str {
        doc["lines"][i]["text"].as_str().unwrap()
    }

    /// Reads a span back out of `text` the way Kotlin does, by UTF-16 unit.
    fn span_text(text: &str, spans: &[i32], n: usize) -> (String, i32) {
        let units: Vec<u16> = text.encode_utf16().collect();
        let [start, end, kind] = spans[n * 3..n * 3 + 3] else {
            panic!("no span {n} in {spans:?}")
        };
        (
            String::from_utf16(&units[start as usize..end as usize]).unwrap(),
            kind,
        )
    }

    #[test]
    fn a_span_after_a_wide_character_is_counted_in_utf16_units() {
        // 😀 is 4 bytes and 2 UTF-16 units; € is 3 bytes and 1 unit.
        let text = "\u{20ac}5 // euros\n\u{1f600} 1 + 1 // smile\n# Heading";
        let spans = highlight(text);
        assert_eq!(spans.len(), 9, "{spans:?}");
        let comment = |n| span_text(text, &spans, n);
        assert!(comment(0).0.contains("// euros") && comment(0).1 == 0);
        assert!(comment(1).0.contains("// smile") && comment(1).1 == 0);
        assert_eq!(comment(2), ("# Heading".to_owned(), 1));
    }

    #[test]
    fn keywords_get_their_own_kind() {
        let text = "1\n2\nsum";
        let spans = highlight(text);
        assert_eq!(span_text(text, &spans, 0), ("sum".to_owned(), 3));
    }

    #[test]
    fn a_result_per_line_with_totals_flagged() {
        let doc = run("5\n\n5\nsum", "[]", false);
        let lines = doc["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 4);
        assert_eq!(line_text(&doc, 0), "5");
        assert!(lines[1].is_null());
        assert_eq!(line_text(&doc, 3), "5");
        assert_eq!(lines[3]["total"], true);
        assert_eq!(lines[0]["total"], false);
    }

    #[test]
    fn the_decimal_comma_is_read_and_shown_swapped() {
        // `,` is thousands by default, so `1,5` is 15.
        assert_eq!(line_text(&run("1,5 + 1", "[]", false), 0), "16");
        let doc = run("1,5 + 1\n1234,5 + 0\n1.000 + 1", "[]", true);
        assert_eq!(line_text(&doc, 0), "2,5");
        assert_eq!(line_text(&doc, 1), "1.234,5");
        // And `.` is thousands: 1.000 is a thousand, not one.
        assert_eq!(line_text(&doc, 2), "1.001");
        assert_eq!(doc["lines"][0]["copy"], "2,5");
    }

    #[test]
    fn converters_apply_and_report_their_status() {
        let converters = r#"[{"unit":"teu","aliases":[],"base":"cbm","factor":"33.2"},
                            {"unit":"bad","aliases":[],"base":"cbm","factor":"x"}]"#;
        let doc = run("2 teu in cbm", converters, false);
        assert_eq!(line_text(&doc, 0), "66.4 m^3");
        assert!(doc["converters"][0]["ok"].is_string(), "{doc}");
        assert!(doc["converters"][1]["error"].is_string(), "{doc}");

        // A factor typed `33,2` with the decimal comma on is 33.2.
        let converters = r#"[{"unit":"teu","aliases":[],"base":"cbm","factor":"33,2"}]"#;
        assert_eq!(
            line_text(&run("2 teu in cbm", converters, true), 0),
            "66,4 m^3"
        );
    }

    #[test]
    fn an_error_carries_its_full_message() {
        let doc = run("5 metr", "[]", false);
        assert_eq!(line_text(&doc, 0), "unknown metr");
        assert_eq!(doc["lines"][0]["error"], "unknown identifier 'metr'");
    }

    #[test]
    fn a_cancelled_recalculation_answers_none() {
        let rates = RateSource::with_rates(&[]);
        assert_eq!(recalc("1 + 1", "[]", false, false, &rates, || true), None);
    }

    #[test]
    fn unreadable_converters_leave_the_lines_working() {
        assert_eq!(line_text(&run("1 + 1", "not json", false), 0), "2");
    }
}

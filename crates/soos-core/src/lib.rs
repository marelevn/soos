//! The whole brain of Soos, no UI: fend-core wrapped with the document model
//! and natural-language layer Numi-style syntax needs on top of it.

pub mod currency;
mod document;
mod engine;
pub mod format;
pub mod highlight;
mod preprocess;

pub use document::{LineResult, RawConverter, MAX_CONVERTERS};

/// Build a fresh, fully-configured fend context: currency handler and
/// soos's own builtin units registered. Call once per recalculation pass
/// (see `document::recalc`'s doc comment for why a full pass is fine).
fn new_context(rates: &currency::RateSource) -> fend_core::Context {
    let mut ctx = fend_core::Context::new();
    ctx.set_exchange_rate_handler_v2(rates.clone());
    engine::register_builtin_units(&mut ctx);
    ctx
}

/// Recalculate a whole document (see `document::recalc`) with currency and
/// the user's own converters wired up -- what soos-app calls on every edit
/// to the document or the converter table. Converters register into the
/// context first, so the document's own lines can use them; their own
/// display-or-error results come back alongside the document's.
pub fn recalc_document(
    source: &str,
    converters: &[RawConverter],
    rates: &currency::RateSource,
) -> (Vec<Result<String, String>>, Vec<LineResult>) {
    let mut ctx = new_context(rates);
    let converter_results = document::define_converters(&mut ctx, converters);
    let line_results = document::recalc(&mut ctx, source);
    (converter_results, line_results)
}

/// Evaluate a single one-shot expression (no document, no `prev`/`sum`,
/// no converters) -- what the CLI and launcher integrations use.
pub fn evaluate_one(expr: &str, rates: &currency::RateSource) -> Result<String, String> {
    let mut ctx = new_context(rates);
    engine::eval_line(&mut ctx, expr).map(|e| e.display)
}

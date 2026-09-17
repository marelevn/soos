//! soos-cli "20 inches in cm" -- one-shot evaluation for scripts and launcher
//! integrations (Alfred, PowerToys Run, Rofi, ...). `--json` gives a
//! machine-readable `{"ok":bool,"result"|"error":string}` on stdout.
//! `--alfred` gives an Alfred Script Filter `{"items":[...]}` and always
//! exits 0 (see integrations/alfred/).

use std::env;

use soos_core::currency::RateSource;

fn main() {
    let mut json = false;
    let mut alfred = false;
    let mut expr_parts = Vec::new();
    for arg in env::args().skip(1) {
        if arg == "--json" {
            json = true;
        } else if arg == "--alfred" {
            alfred = true;
        } else {
            expr_parts.push(arg);
        }
    }
    let expr = expr_parts.join(" ");
    if expr.trim().is_empty() {
        eprintln!("usage: soos-cli [--json|--alfred] \"<expression>\"");
        std::process::exit(2);
    }

    let rates = RateSource::new(soos_core::currency::default_cache_path());
    // One-shot CLI, so blocking once at startup (only if the cache is
    // actually stale) is the right call, unlike soos-app which must never
    // block its UI thread -- see refresh_in_background there.
    rates.refresh_if_stale_blocking();

    let outcome = soos_core::evaluate_one(&expr, &rates);

    // Alfred Script Filters show Alfred's own error sheet on a non-zero exit
    // instead of rendering the row -- the error belongs in the row's title,
    // so this mode always exits 0.
    if alfred {
        println!(
            "{}",
            alfred_json(outcome.as_deref().map_err(String::as_str), &expr)
        );
        return;
    }

    match outcome {
        Ok(result) => {
            if json {
                println!("{}", serde_json::json!({"ok": true, "result": result}));
            } else {
                println!("{result}");
            }
        }
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({"ok": false, "error": e}));
            } else {
                eprintln!("{e}");
            }
            std::process::exit(1);
        }
    }
}

/// Alfred Script Filter JSON: https://www.alfredapp.com/help/workflows/inputs/script-filter/json/
/// One item, no `uid` -- with a single row it buys nothing and Alfred's
/// result-learning would reorder it oddly across runs.
fn alfred_json(outcome: Result<&str, &str>, expr: &str) -> String {
    let item = match outcome {
        Ok(result) => serde_json::json!({
            "title": result,
            "subtitle": expr,
            "arg": result,
            "valid": true,
            "text": {"copy": result, "largetype": result},
        }),
        Err(e) => serde_json::json!({
            "title": e,
            "subtitle": expr,
            "valid": false,
        }),
    };
    serde_json::json!({"items": [item]}).to_string()
}

#[cfg(test)]
mod tests {
    use super::alfred_json;

    #[test]
    fn alfred_json_success_shape() {
        let out = alfred_json(Ok("50.8 cm"), "20 inches in cm");
        assert!(out.contains("\"title\":\"50.8 cm\""));
        assert!(out.contains("\"subtitle\":\"20 inches in cm\""));
        assert!(out.contains("\"valid\":true"));
        assert!(out.contains("\"arg\":\"50.8 cm\""));
    }

    #[test]
    fn alfred_json_error_shape_stays_exit_zero() {
        let out = alfred_json(Err("parse error"), "1 +");
        assert!(out.contains("\"title\":\"parse error\""));
        assert!(out.contains("\"valid\":false"));
        assert!(!out.contains("\"arg\""));
    }

    #[test]
    fn alfred_json_escapes_quotes_in_expression() {
        let out = alfred_json(Ok("42"), "say \"hi\"");
        assert!(out.contains("\"subtitle\":\"say \\\"hi\\\"\""));
    }
}

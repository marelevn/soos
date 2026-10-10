//! `soos-cli "20 inches in cm"`, for scripts and the launcher integrations
//! (`integrations/`). It prints what the app shows beside the same line,
//! using the app's converters: see [`soos_core::evaluate_one`], and
//! [`USAGE`] for the options.

use std::io::Read;
use std::process::ExitCode;

use soos_core::currency::RateSource;
use soos_core::format::Shown;
use soos_core::storage;

const USAGE: &str = "\
usage: soos-cli [options] <expression>...
       soos-cli [options] -       read the input from stdin

Prints what the Soos app shows beside the same line: currency symbols,
rounding and thousands separators included. Uses the app's exchange-rate
cache and the converters defined in the app.

options:
  --json             print {\"ok\":true,\"result\":...,\"value\":...} or
                     {\"ok\":false,\"error\":...,\"detail\":...}; \"value\" is the
                     result without thousands separators, \"detail\" the full
                     error message
  --high-precision   keep every digit instead of rounding currencies
                     (the app's high-precision toggle)
  -h, --help         show this help
  -V, --version      show the version
  --                 treat everything after this as the expression

exit status: 0 result, 1 error or nothing to calculate, 2 usage error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Plain,
    Json,
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    format: Format,
    high_precision: bool,
    /// `None` means "read stdin" (`-`).
    expression: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run(Options),
    Help,
    Version,
    /// A usage mistake, with what went wrong.
    Usage(String),
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Command {
    let mut format = Format::Plain;
    let mut high_precision = false;
    let mut parts: Vec<String> = Vec::new();
    let mut options_done = false;
    for arg in args {
        if options_done {
            parts.push(arg);
            continue;
        }
        match arg.as_str() {
            "--json" => format = Format::Json,
            "--high-precision" => high_precision = true,
            "-h" | "--help" => return Command::Help,
            "-V" | "--version" => return Command::Version,
            "--" => options_done = true,
            // `-` and `-5 + 3` are input; only an unknown `--word` is a
            // mistake.
            other if other.starts_with("--") => {
                return Command::Usage(format!("unknown option {other}"));
            }
            _ => parts.push(arg),
        }
    }
    // A lone `-` reads stdin; among other words it is a minus (`5 - 3`).
    let expression = match parts.as_slice() {
        [only] if only == "-" => None,
        _ => Some(parts.join(" ")),
    };
    Command::Run(Options {
        format,
        high_precision,
        expression,
    })
}

/// What to print and the exit status, apart from printing so it's testable.
#[derive(Debug, PartialEq, Eq)]
struct Output {
    stdout: Option<String>,
    stderr: Option<String>,
    code: u8,
}

/// Input with nothing to calculate (`shown` is `None`) is an error, so every
/// call gives one result or one error.
fn render(format: Format, shown: Option<Shown>) -> Output {
    let shown = shown.unwrap_or_else(|| Shown {
        text: "nothing to calculate".to_string(),
        copy: "nothing to calculate".to_string(),
        error: Some("only blank lines, comments, headers or labels".to_string()),
        full: None,
    });
    let code = u8::from(shown.error.is_some());
    match format {
        Format::Plain => match &shown.error {
            None => Output {
                stdout: Some(shown.text),
                stderr: None,
                code,
            },
            // The short label, then the full message if it says more.
            Some(detail) if *detail != shown.text => Output {
                stdout: None,
                stderr: Some(format!("{}\n  {detail}", shown.text)),
                code,
            },
            Some(_) => Output {
                stdout: None,
                stderr: Some(shown.text),
                code,
            },
        },
        Format::Json => {
            let json = match &shown.error {
                None => serde_json::json!({"ok": true, "result": shown.text, "value": shown.copy}),
                Some(detail) => {
                    serde_json::json!({"ok": false, "error": shown.text, "detail": detail})
                }
            };
            Output {
                stdout: Some(json.to_string()),
                stderr: None,
                code,
            }
        }
    }
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Command::Run(options) => options,
        Command::Help => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("soos-cli {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Usage(problem) => {
            eprintln!("soos-cli: {problem}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let input = match options.expression {
        Some(expression) => expression,
        None => {
            let mut input = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut input) {
                eprintln!("soos-cli: can't read stdin: {e}");
                return ExitCode::from(2);
            }
            input
        }
    };
    if input.trim().is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    let rates = RateSource::new(soos_core::currency::default_cache_path());
    // Blocks for at most a few seconds, and not at all within a few minutes
    // of a failed attempt, so an offline launcher stays responsive.
    rates.refresh_if_stale_blocking();
    let converters = storage::load_converters(&storage::converters_path());

    let shown = soos_core::evaluate_one(&input, &converters, &rates, options.high_precision);
    let output = render(options.format, shown);
    if let Some(stdout) = output.stdout {
        println!("{stdout}");
    }
    if let Some(stderr) = output.stderr {
        eprintln!("{stderr}");
    }
    ExitCode::from(output.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Command {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    fn value(text: &str, copy: &str) -> Option<Shown> {
        Some(Shown {
            text: text.to_string(),
            copy: copy.to_string(),
            error: None,
            full: None,
        })
    }

    fn error(short: &str, detail: &str) -> Option<Shown> {
        Some(Shown {
            text: short.to_string(),
            copy: short.to_string(),
            error: Some(detail.to_string()),
            full: None,
        })
    }

    #[test]
    fn parses_flags_anywhere_and_joins_the_rest() {
        assert_eq!(
            args(&["20", "inches", "--json", "in", "cm", "--high-precision"]),
            Command::Run(Options {
                format: Format::Json,
                high_precision: true,
                expression: Some("20 inches in cm".to_string()),
            })
        );
    }

    #[test]
    fn negative_numbers_and_double_dash_are_input_not_options() {
        let Command::Run(options) = args(&["-5", "+", "3"]) else {
            panic!("expected Run");
        };
        assert_eq!(options.expression.as_deref(), Some("-5 + 3"));
        let Command::Run(options) = args(&["--", "--json"]) else {
            panic!("expected Run");
        };
        assert_eq!(options.expression.as_deref(), Some("--json"));
        assert_eq!(options.format, Format::Plain);
    }

    #[test]
    fn dash_alone_reads_stdin_and_mistakes_are_usage_errors() {
        let Command::Run(options) = args(&["--json", "-"]) else {
            panic!("expected Run");
        };
        assert_eq!(options.expression, None);
        // Among other words a `-` is a minus, as an unquoted `5 - 3` is.
        let Command::Run(options) = args(&["5", "-", "3"]) else {
            panic!("expected Run");
        };
        assert_eq!(options.expression.as_deref(), Some("5 - 3"));
        assert!(matches!(args(&["--jsn", "1"]), Command::Usage(_)));
        assert_eq!(args(&["-h"]), Command::Help);
        assert_eq!(args(&["--version"]), Command::Version);
    }

    #[test]
    fn plain_prints_the_app_text_or_the_label_and_detail() {
        let ok = render(Format::Plain, value("$2,469.00", "$2469.00"));
        assert_eq!(ok.stdout.as_deref(), Some("$2,469.00"));
        assert_eq!(ok.code, 0);

        let err = render(
            Format::Plain,
            error("unknown metr", "unknown identifier 'metr'"),
        );
        assert_eq!(err.stdout, None);
        assert_eq!(
            err.stderr.as_deref(),
            Some("unknown metr\n  unknown identifier 'metr'")
        );
        assert_eq!(err.code, 1);
    }

    #[test]
    fn json_separates_what_is_shown_from_the_plain_value() {
        let ok = render(Format::Json, value("$2,469.00", "$2469.00"));
        let json: serde_json::Value = serde_json::from_str(ok.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["result"], "$2,469.00");
        assert_eq!(json["value"], "$2469.00");

        let err = render(Format::Json, error("syntax error", "found ')'"));
        let json: serde_json::Value = serde_json::from_str(err.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"], "syntax error");
        assert_eq!(json["detail"], "found ')'");
        assert_eq!(err.code, 1);
    }

    #[test]
    fn nothing_to_calculate_is_an_error() {
        let out = render(Format::Json, None);
        let json: serde_json::Value = serde_json::from_str(out.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["error"], "nothing to calculate");
        assert_eq!(out.code, 1);
    }

    /// 1 EUR = 1 USD.
    fn fixed_rates() -> RateSource {
        RateSource::with_rates(&[("EUR", 1.0), ("USD", 1.0)])
    }

    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Runs `lines`, "expression  result" pairs as in the README's Quick
    /// guide, as one document, and checks every result given.
    fn assert_results(name: &str, lines: &[&str], rates: &RateSource) {
        let pairs: Vec<(&str, Option<&str>)> = lines
            .iter()
            .map(|l| match l.trim().rsplit_once("  ") {
                Some((expr, result)) => (expr.trim(), Some(result.trim())),
                None => (l.trim(), None),
            })
            .collect();
        let document: Vec<&str> = pairs.iter().map(|(expr, _)| *expr).collect();
        let (_, results) = soos_core::recalc_document(&document.join("\n"), &[], rates);
        for ((expr, expected), result) in pairs.iter().zip(&results) {
            if let Some(expected) = expected {
                let shown = soos_core::format::shown(result, false).map(|s| s.text);
                assert_eq!(shown.as_deref(), Some(*expected), "{name}: {expr}");
            }
        }
    }

    /// Every result shown in the README and the docs pages, except dates,
    /// which depend on today: the README's Quick guide (run as one
    /// document) and the soos-cli examples.
    #[test]
    fn readme_examples_match_the_engine() {
        let root = repo_root();
        let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
        let rates = fixed_rates();

        let quick = &readme[readme.find("## Quick guide").unwrap()..];
        let block: Vec<&str> = quick
            .lines()
            .skip_while(|l| !l.starts_with("    "))
            .take_while(|l| l.starts_with("    ") || l.is_empty())
            .collect();
        assert!(block.len() > 3, "no Quick guide example found");
        assert_results("README", &block, &rates);

        let mut pages = vec![("README.md".to_string(), readme)];
        for entry in std::fs::read_dir(root.join("docs")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "md") {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                pages.push((name, std::fs::read_to_string(&path).unwrap()));
            }
        }
        let mut checked = 0;
        for (page, text) in &pages {
            let lines: Vec<&str> = text.lines().collect();
            for pair in lines.windows(2) {
                let Some(expr) = pair[0]
                    .strip_prefix("$ soos-cli '")
                    .and_then(|rest| rest.strip_suffix('\''))
                else {
                    continue;
                };
                let expected = pair[1].trim();
                // A date or time reads `Friday, 25 December 2026`, and
                // these depend on today.
                if [
                    "Monday,",
                    "Tuesday,",
                    "Wednesday,",
                    "Thursday,",
                    "Friday,",
                    "Saturday,",
                    "Sunday,",
                ]
                .iter()
                .any(|day| expected.starts_with(day))
                {
                    continue;
                }
                let shown = soos_core::evaluate_one(expr, &[], &rates, false).map(|s| s.text);
                assert_eq!(shown.as_deref(), Some(expected), "{page}: {expr}");
                checked += 1;
            }
        }
        assert!(checked >= 2, "no soos-cli examples found");
    }

    /// The cards the docs pages show (docs/cards/*.txt, drawn by
    /// scripts/make-cards.py): each is a document with its results beside
    /// it, so a card can't show what the engine doesn't print.
    #[test]
    fn doc_cards_match_the_engine() {
        let rates = fixed_rates();
        let mut checked = 0;
        for entry in std::fs::read_dir(repo_root().join("docs/cards")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "txt") {
                let text = std::fs::read_to_string(&path).unwrap();
                let lines: Vec<&str> = text.lines().collect();
                assert_results(&path.display().to_string(), &lines, &rates);
                checked += 1;
            }
        }
        assert!(checked >= 5, "no cards found");
    }

    #[test]
    fn cli_output_matches_the_app_for_a_real_line() {
        let rates = RateSource::new(std::path::PathBuf::new());
        let shown = soos_core::evaluate_one("Area: 1200 * 3 m^2 // hall", &[], &rates, false);
        let out = render(Format::Plain, shown);
        assert_eq!(out.stdout.as_deref(), Some("3,600 m^2"));
    }
}

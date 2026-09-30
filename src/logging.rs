//! Minimal `log` backend that reproduces tracing-subscriber's default
//! `fmt` output (without timestamps) and its `RUST_LOG` target directives.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::io::Write;

struct Directive {
    target: Option<String>,
    level: LevelFilter,
}

struct Logger {
    /// Most specific (longest target) first; bare levels last.
    directives: Vec<Directive>,
    ansi: bool,
}

pub fn init() {
    let directives = std::env::var("RUST_LOG")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| parse_directives(&s))
        .unwrap_or_else(|| vec![Directive { target: None, level: LevelFilter::Warn }]);
    let max = directives.iter().map(|d| d.level).max().unwrap_or(LevelFilter::Off);
    let ansi = std::env::var("NO_COLOR").map_or(true, |v| v.is_empty());
    let logger: &'static Logger = Box::leak(Box::new(Logger { directives, ansi }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(max);
    }
}

impl Logger {
    fn level_for(&self, target: &str) -> LevelFilter {
        self.directives
            .iter()
            .find(|d| d.target.as_deref().is_none_or(|t| target.starts_with(t)))
            .map_or(LevelFilter::Off, |d| d.level)
    }
}

impl Log for Logger {
    fn enabled(&self, meta: &Metadata) -> bool {
        meta.level() <= self.level_for(meta.target())
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let (color, label) = match record.level() {
            Level::Error => (31, "ERROR"),
            Level::Warn => (33, " WARN"),
            Level::Info => (32, " INFO"),
            Level::Debug => (34, "DEBUG"),
            Level::Trace => (35, "TRACE"),
        };
        let target = record.target();
        let args = record.args();
        let line = if self.ansi {
            format!("\x1b[{color}m{label}\x1b[0m \x1b[2m{target}\x1b[0m\x1b[2m:\x1b[0m {args}\n")
        } else {
            format!("{label} {target}: {args}\n")
        };
        let _ = std::io::stdout().lock().write_all(line.as_bytes());
    }

    fn flush(&self) {
        let _ = std::io::stdout().flush();
    }
}

/// Same grammar as tracing-subscriber's `Targets`: comma-separated
/// `target=level`, bare `level` or bare `target` (meaning trace); a later
/// directive for the same target replaces an earlier one.
fn parse_directives(spec: &str) -> Option<Vec<Directive>> {
    let mut out: Vec<Directive> = Vec::new();
    for part in spec.split(',') {
        let directive = match part.split_once('=') {
            Some((target, level)) => {
                if level.contains('=') {
                    return None;
                }
                let target = match target.split_once("[{") {
                    Some((t, fields)) if fields.ends_with("}]") && !fields.contains("[{") => {
                        // Every log record carries exactly one field, `message`.
                        let only_message = fields.trim_end_matches("}]").split(',').all(|f| f.is_empty() || f == "message");
                        if !only_message {
                            continue;
                        }
                        t
                    }
                    Some(_) => return None,
                    None => target,
                };
                Directive { target: Some(target.to_string()), level: parse_level(level)? }
            }
            None => match parse_level(part) {
                Some(level) => Directive { target: None, level },
                None => Directive { target: Some(part.to_string()), level: LevelFilter::Trace },
            },
        };
        out.retain(|d| d.target != directive.target);
        out.push(directive);
    }
    out.sort_by(|a, b| b.target.as_ref().map(String::len).cmp(&a.target.as_ref().map(String::len)));
    Some(out)
}

fn parse_level(s: &str) -> Option<LevelFilter> {
    Some(match s {
        "0" => LevelFilter::Off,
        "" | "1" => LevelFilter::Error,
        "2" => LevelFilter::Warn,
        "3" => LevelFilter::Info,
        "4" => LevelFilter::Debug,
        "5" => LevelFilter::Trace,
        s if s.eq_ignore_ascii_case("off") => LevelFilter::Off,
        s if s.eq_ignore_ascii_case("error") => LevelFilter::Error,
        s if s.eq_ignore_ascii_case("warn") => LevelFilter::Warn,
        s if s.eq_ignore_ascii_case("info") => LevelFilter::Info,
        s if s.eq_ignore_ascii_case("debug") => LevelFilter::Debug,
        s if s.eq_ignore_ascii_case("trace") => LevelFilter::Trace,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logger(spec: &str) -> Logger {
        Logger { directives: parse_directives(spec).expect("valid"), ansi: false }
    }

    #[test]
    fn most_specific_target_wins() {
        let l = logger("warn,xrs=debug,xrs::storage=error");
        assert_eq!(l.level_for("xrs::ui"), LevelFilter::Debug);
        assert_eq!(l.level_for("xrs::storage"), LevelFilter::Error);
        assert_eq!(l.level_for("ureq::run"), LevelFilter::Warn);
    }

    #[test]
    fn bare_target_means_trace_and_nothing_else() {
        let l = logger("ureq");
        assert_eq!(l.level_for("ureq::run"), LevelFilter::Trace);
        assert_eq!(l.level_for("xrs"), LevelFilter::Off);
    }

    #[test]
    fn grammar_edge_cases() {
        assert!(parse_directives("a=b=c").is_none());
        assert!(parse_directives("xrs=loud").is_none());
        assert_eq!(logger("xrs=").level_for("xrs"), LevelFilter::Error);
        assert_eq!(logger("3").level_for("x"), LevelFilter::Info);
        assert_eq!(logger("xrs=info,xrs=trace").level_for("xrs"), LevelFilter::Trace);
    }
}

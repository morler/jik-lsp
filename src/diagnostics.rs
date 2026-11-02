//! Parsing for the current human-readable `jik check` diagnostic format.
//!
//! The compiler reports 1-based lines and UTF-8 byte columns, but does not
//! render the end of its internal span. Do not treat the point as a full span.

use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub struct CompilerDiagnostic {
    pub path: PathBuf,
    pub line: usize,
    pub byte_column: usize,
    pub message: String,
    pub warning: bool,
}

/// Preserve diagnostics without locations separately; they must not become
/// fabricated source ranges. Parse paths from the right to allow colons.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedDiagnostics {
    pub located: Vec<CompilerDiagnostic>,
    pub unlocated: Vec<String>,
}

type PendingDiagnostic = (String, bool, Option<(PathBuf, usize, usize)>);

pub fn parse(output: &str) -> ParsedDiagnostics {
    let mut result = ParsedDiagnostics::default();
    let mut current: Option<PendingDiagnostic> = None;
    fn finish(result: &mut ParsedDiagnostics, current: &mut Option<PendingDiagnostic>) {
        if let Some((message, warning, location)) = current.take() {
            if let Some((path, line, byte_column)) = location {
                result.located.push(CompilerDiagnostic {
                    path,
                    line,
                    byte_column,
                    message,
                    warning,
                });
            } else {
                result.unlocated.push(message);
            }
        }
    }
    for line in output.lines() {
        let header = line
            .strip_prefix("jik error: ")
            .map(|s| (s, false))
            .or_else(|| line.strip_prefix("jik warning: ").map(|s| (s, true)));
        if let Some((message, warning)) = header {
            finish(&mut result, &mut current);
            current = Some((message.to_owned(), warning, None));
        } else if let Some((message, _, location)) = current.as_mut() {
            if let Some(raw) = line.trim_start().strip_prefix("--> ") {
                *location = parse_location(raw);
            } else if let Some(detail) = line.trim_start().strip_prefix("= ") {
                message.push_str(": ");
                message.push_str(detail);
            }
        }
    }
    finish(&mut result, &mut current);
    result
}

fn parse_location(location: &str) -> Option<(PathBuf, usize, usize)> {
    let (path_and_line, column) = location.rsplit_once(':')?;
    let (path, line) = path_and_line.rsplit_once(':')?;
    let line = line.parse::<usize>().ok()?.checked_sub(1)?;
    let column = column.parse::<usize>().ok()?.checked_sub(1)?;
    if path.is_empty() {
        return None;
    }
    Some((PathBuf::from(path), line, column))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_multiple_messages_and_colon_paths() {
        let output = "jik error: semantic analysis\n      --> C:\\a b\\x.jik:2:12\n  |\n2 |    return unknown\n  |           ^\n    = undefined symbol: unknown\njik warning: notice\n      --> /tmp/a:b.jik:1:1\njik error: no location\n    explanation\n";
        let parsed = parse(output);
        assert_eq!(parsed.located.len(), 2);
        assert_eq!(
            parsed.located[0],
            CompilerDiagnostic {
                path: PathBuf::from("C:\\a b\\x.jik"),
                line: 1,
                byte_column: 11,
                message: "semantic analysis: undefined symbol: unknown".into(),
                warning: false
            }
        );
        assert_eq!(parsed.located[1].path, PathBuf::from("/tmp/a:b.jik"));
        assert!(parsed.located[1].warning);
        assert_eq!(parsed.unlocated, ["no location"]);
    }
    #[test]
    fn rejects_invalid_locations() {
        for location in [
            ":1:1",
            "x:0:1",
            "x:1:0",
            "x:a:2",
            "x:1",
            "x:1:99999999999999999999999999",
        ] {
            assert!(parse_location(location).is_none(), "{location}");
        }
        assert_eq!(parse("jik error: bad\n --> x:0:1\n").unlocated, ["bad"]);
    }
}

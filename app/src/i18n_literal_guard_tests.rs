//! Source guard (#516): user-visible English must not be hard-coded in the
//! cockpit's formatting spine or in the Connections panel. Their words belong
//! in the Fluent catalogs (`app/i18n/*/warp.ftl`), reached through `t!`, or —
//! for the language-neutral `zaplex_cockpit` crate — in structured values the
//! app localizes (`cockpit::localize`).
//!
//! The scanner reads every string literal outside comments and flags one that
//! carries words (two or more ASCII letters) or a unit/currency glued to a
//! format placeholder (`{n}m`, `~${usd}`). It never flags:
//! - diagnostics and ids: arguments of log macros, assertions, panics,
//!   `expect`, `t!`/`t_static!` (Fluent message ids) and `id!`, plus attributes;
//! - identifier-shaped literals: no whitespace and a `_ : / . @ # = -`, a digit
//!   or an inner capital (`ssh-manager-node:{id}`, `cmdorctrl-r`, `5h`);
//! - key parts joined with `\0`;
//! - words a file legitimately carries as data, listed with their reason below.
//!
//! A new plain-word identifier in these files needs an entry in its allowlist;
//! a new UI word needs a catalog entry instead.

use regex::Regex;

/// Callees whose string arguments are never UI copy.
const NON_UI_CALLEES: &[&str] = &[
    "error!",
    "warn!",
    "info!",
    "debug!",
    "trace!",
    "assert!",
    "assert_eq!",
    "assert_ne!",
    "debug_assert!",
    "debug_assert_eq!",
    "debug_assert_ne!",
    "panic!",
    "unreachable!",
    "todo!",
    "unimplemented!",
    "expect",
    "t!",
    "t_static!",
    "id!",
    "include_str!",
];

/// Model families and model-id fragments matched against transcript ids
/// (proper names, not copy), plus the binding-window id `wk`.
const FORMAT_ALLOWED: &[&str] = &["opus", "sonnet", "haiku", "fable", "gpt", "codex", "wk"];
/// Components of session identity keys (`local`, `opaque`, `default`,
/// `unknown`, `legacy`) and the model families of the model·effort label.
const CONDUCTOR_ALLOWED: &[&str] = &[
    "local", "opaque", "default", "unknown", "legacy", "opus", "sonnet", "haiku", "fable",
];
/// The daemon runtime key `current`, the `tailscale status` command line,
/// keymap key names, and the multiplexer product names.
const PANEL_ALLOWED: &[&str] = &[
    "current",
    "tailscale",
    "status",
    "up",
    "down",
    "enter",
    "left",
    "right",
    "tmux",
    "byobu",
    "screen",
];

#[derive(Debug)]
struct Literal {
    line: usize,
    text: String,
    callees: Vec<String>,
}

/// Reads the body of a normal string literal starting after its opening quote;
/// escapes stay as written. Returns the body and the index after the closing
/// quote.
fn escaped_body(chars: &[char], start: usize) -> (String, usize) {
    let mut body = String::new();
    let mut index = start;
    while index < chars.len() && chars[index] != '"' {
        if chars[index] == '\\' && index + 1 < chars.len() {
            body.push(chars[index]);
            body.push(chars[index + 1]);
            index += 2;
        } else {
            body.push(chars[index]);
            index += 1;
        }
    }
    (body, index + 1)
}

/// Reads the body of a raw string literal with `hashes` `#`s.
fn raw_body(chars: &[char], start: usize, hashes: usize) -> (String, usize) {
    let mut index = start;
    while index < chars.len() {
        if chars[index] == '"' && (1..=hashes).all(|k| chars.get(index + k) == Some(&'#')) {
            return (chars[start..index].iter().collect(), index + 1 + hashes);
        }
        index += 1;
    }
    (chars[start..].iter().collect(), chars.len())
}

/// Every string literal outside comments, with the callees of the calls and
/// macros it sits in (`#[` for attributes).
fn string_literals(source: &str) -> Vec<Literal> {
    let chars: Vec<char> = source.chars().collect();
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let starts_with = |at: usize, pattern: &str| {
        pattern
            .chars()
            .enumerate()
            .all(|(offset, expected)| chars.get(at + offset) == Some(&expected))
    };
    let mut literals = Vec::new();
    let mut callees: Vec<String> = Vec::new();
    let mut last_ident = String::new();
    let mut line = 1;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let after_ident = index > 0 && is_ident(chars[index - 1]);
        if c == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if starts_with(index, "//") {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if starts_with(index, "/*") {
            let mut depth = 0;
            while index < chars.len() {
                if starts_with(index, "/*") {
                    depth += 1;
                    index += 2;
                } else if starts_with(index, "*/") {
                    depth -= 1;
                    index += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[index] == '\n' {
                        line += 1;
                    }
                    index += 1;
                }
            }
            continue;
        }
        if c == '"' || (!after_ident && (c == 'r' || c == 'b')) {
            let mut start = index;
            let is_bytes = chars[start] == 'b';
            if is_bytes {
                start += 1;
            }
            let is_raw = chars.get(start) == Some(&'r');
            if is_raw {
                start += 1;
            }
            let mut hashes = 0;
            while is_raw && chars.get(start) == Some(&'#') {
                hashes += 1;
                start += 1;
            }
            if chars.get(start) == Some(&'"') {
                let (text, end) = if is_raw {
                    raw_body(&chars, start + 1, hashes)
                } else {
                    escaped_body(&chars, start + 1)
                };
                let lines = text.matches('\n').count();
                if !is_bytes {
                    literals.push(Literal {
                        line,
                        text,
                        callees: callees.clone(),
                    });
                }
                line += lines;
                index = end;
                last_ident.clear();
                continue;
            }
        }
        if c == '\'' {
            if chars.get(index + 1) == Some(&'\\') {
                index += 3;
                while index < chars.len() && chars[index] != '\'' {
                    index += 1;
                }
                index += 1;
            } else if chars.get(index + 2) == Some(&'\'') {
                index += 3;
            } else {
                index += 1;
            }
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = index;
            while index < chars.len() && is_ident(chars[index]) {
                index += 1;
            }
            last_ident = chars[start..index].iter().collect();
            if chars.get(index) == Some(&'!') {
                last_ident.push('!');
                index += 1;
            }
            continue;
        }
        match c {
            '(' => callees.push(std::mem::take(&mut last_ident)),
            '[' => {
                let attribute = index > 0
                    && (chars[index - 1] == '#'
                        || (chars[index - 1] == '!' && index > 1 && chars[index - 2] == '#'));
                callees.push(if attribute { "#[" } else { "[" }.to_string());
            }
            '{' => callees.push("{".to_string()),
            ')' | ']' | '}' => {
                callees.pop();
            }
            _ => {}
        }
        if !c.is_whitespace() {
            last_ident.clear();
        }
        index += 1;
    }
    literals
}

/// The literals of `source` that read as hard-coded English UI copy, as
/// `(line, literal)`.
fn english_ui_literals(source: &str, allowed_words: &[&str]) -> Vec<(usize, String)> {
    let escape = Regex::new(r"\\u\{[0-9a-fA-F]+\}|\\.").expect("valid escape regex");
    let placeholder = Regex::new(r"\{[^{}]*\}").expect("valid placeholder regex");
    let glued_unit = Regex::new(r"\}[A-Za-z]").expect("valid unit regex");
    let glued_currency = Regex::new(r"\$\{|\}\$").expect("valid currency regex");
    let identifier_mark = Regex::new(r"[_:/.@#=0-9-]").expect("valid identifier regex");
    let word = Regex::new(r"[A-Za-z]{2,}").expect("valid word regex");
    string_literals(source)
        .into_iter()
        .filter(|literal| {
            !literal
                .callees
                .iter()
                .any(|callee| callee == "#[" || NON_UI_CALLEES.contains(&callee.as_str()))
        })
        .filter(|literal| !literal.text.contains("\\0") && !literal.text.contains("\\u{0}"))
        .filter(|literal| {
            let unescaped = escape.replace_all(&literal.text, " ");
            let glued = glued_unit.is_match(&unescaped) || glued_currency.is_match(&unescaped);
            let without_placeholders = placeholder.replace_all(&unescaped, "\u{1}");
            let compact = without_placeholders.trim();
            let identifier_shaped = !compact.is_empty()
                && !compact.chars().any(char::is_whitespace)
                && (identifier_mark.is_match(compact)
                    || compact.chars().skip(1).any(char::is_uppercase));
            if glued {
                return true;
            }
            !identifier_shaped
                && word.find_iter(compact).any(|found| {
                    !allowed_words.contains(&found.as_str().to_ascii_lowercase().as_str())
                })
        })
        .map(|literal| (literal.line, literal.text))
        .collect()
}

#[test]
fn cockpit_and_connections_sources_have_no_hard_coded_english_ui_literals() {
    let sources = [
        (
            "crates/zaplex_cockpit/src/format.rs",
            include_str!("../../crates/zaplex_cockpit/src/format.rs"),
            FORMAT_ALLOWED,
        ),
        (
            "crates/zaplex_cockpit/src/conductor.rs",
            include_str!("../../crates/zaplex_cockpit/src/conductor.rs"),
            CONDUCTOR_ALLOWED,
        ),
        (
            "app/src/ssh_manager/panel.rs",
            include_str!("ssh_manager/panel.rs"),
            PANEL_ALLOWED,
        ),
    ];
    let findings: Vec<_> = sources
        .iter()
        .flat_map(|(path, source, allowed)| {
            english_ui_literals(source, allowed)
                .into_iter()
                .map(move |(line, literal)| format!("{path}:{line}: {literal:?}"))
        })
        .collect();
    assert!(
        findings.is_empty(),
        "hard-coded English UI literals; move them into the Fluent catalogs:\n{}",
        findings.join("\n")
    );
}

/// The guard must catch the very literals #516 replaced — verbatim snippets of
/// the old code — and must not flag diagnostics, message ids, attributes or
/// identifiers around them.
#[test]
fn literal_guard_flags_the_english_it_replaced_and_ignores_non_ui_strings() {
    let replaced = [
        r#"fn f() -> String { "unpriced".to_string() }"#,
        r#"fn f(usd: f64) -> String { format!("~${usd:.2}") }"#,
        r#"fn f() -> String { return "resetting".to_string(); }"#,
        r#"fn f(total_min: i64) -> String { format!("{total_min}m") }"#,
        r#"fn f(h: i64, m: i64) -> String { format!("{}h{}m", h, m) }"#,
        r#"fn f() -> String { "now".to_string() }"#,
        r#"fn f(s: S) -> &'static str { match s { S::Waiting => "waiting", S::Idle => "idle" } }"#,
        r#"fn f(pct: u32) -> String { format!("{pct}% ctx") }"#,
        r#"fn f(c: &mut C) { let name = unique_name(c, None, "New server")?; }"#,
        r#"fn f() -> &'static str { match k { Err(_) => "unknown" } }"#,
        r#"fn f() -> String { "Tailscale discovery timed out.".to_string() }"#,
    ];
    for snippet in replaced {
        assert!(
            !english_ui_literals(snippet, PANEL_ALLOWED).is_empty(),
            "missed {snippet}"
        );
    }

    let not_ui = [
        r#"fn f(e: E) { log::error!("ssh_manager: create folder failed: {e:?}"); }"#,
        r#"fn f() -> String { crate::t!("workspace-left-panel-ssh-manager-menu-new-server") }"#,
        "#[cfg(test)]\n#[path = \"panel_tests.rs\"]\nmod tests;",
        r#"fn f(id: &str) -> String { format!("ssh-manager-node:{id}") }"#,
        r#"fn f(x: Option<X>) -> X { x.expect("fixture has no in-flight fetch") }"#,
        r#"fn f(h: &str) -> String { format!("{}\u{0}{h}", "local") }"#,
        r#"// A comment may say "New server" freely."#,
        r##"fn f() -> &'static str { "#22c55e" }"##,
        r#"fn f() -> (f64, &'static str) { (0.0, "5h") }"#,
        r#"fn f() -> String { format!("{}\u{202f}%", 62) }"#,
    ];
    for snippet in not_ui {
        assert_eq!(
            english_ui_literals(snippet, CONDUCTOR_ALLOWED),
            Vec::<(usize, String)>::new(),
            "flagged {snippet}"
        );
    }
}

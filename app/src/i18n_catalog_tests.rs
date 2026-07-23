fn fluent_value(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let variant = line.strip_prefix("*[").or_else(|| line.strip_prefix('['));
    if let Some(variant) = variant {
        return variant.split_once(']').map(|(_, value)| value);
    }

    if let Some((identifier, value)) = line.split_once('=') {
        let identifier = identifier.trim();
        let is_fluent_identifier = !identifier.is_empty()
            && identifier.chars().all(|character| {
                character.is_alphanumeric() || matches!(character, '-' | '_' | '.')
            });
        if is_fluent_identifier {
            return Some(value);
        }
    }

    Some(line)
}

fn is_visible_character_before_ellipsis(character: char) -> bool {
    !character.is_whitespace()
}

fn strip_trailing_fluent_whitespace_placeable(value: &str) -> Option<&str> {
    let before_closing_brace = value.strip_suffix('}')?.trim_end();
    let before_closing_quote = before_closing_brace.strip_suffix('"')?;
    let opening_quote = before_closing_quote.rfind('"')?;
    let whitespace = &before_closing_quote[opening_quote + 1..];
    if whitespace.is_empty() || !whitespace.chars().all(char::is_whitespace) {
        return None;
    }

    before_closing_quote[..opening_quote]
        .trim_end()
        .strip_suffix('{')
}

fn has_space_before_ellipsis(line: &str) -> bool {
    let Some(value) = fluent_value(line) else {
        return false;
    };

    value.char_indices().any(|(ellipsis_index, character)| {
        let is_ellipsis = character == '…' || value[ellipsis_index..].starts_with("...");
        if !is_ellipsis {
            return false;
        }

        let mut prefix = &value[..ellipsis_index];
        let mut removed_whitespace = false;
        loop {
            while let Some((index, previous)) = prefix.char_indices().next_back() {
                if !previous.is_whitespace() {
                    break;
                }
                removed_whitespace = true;
                prefix = &prefix[..index];
            }
            let Some(without_placeable) = strip_trailing_fluent_whitespace_placeable(prefix) else {
                break;
            };
            removed_whitespace = true;
            prefix = without_placeable;
        }

        removed_whitespace
            && prefix
                .chars()
                .next_back()
                .is_some_and(is_visible_character_before_ellipsis)
    })
}

#[test]
fn ui_copy_has_no_space_before_ellipsis() {
    let catalogs = [
        ("en", include_str!("../i18n/en/warp.ftl")),
        ("de", include_str!("../i18n/de/warp.ftl")),
    ];
    let violations: Vec<_> = catalogs
        .iter()
        .copied()
        .flat_map(|(locale, catalog)| {
            catalog
                .lines()
                .enumerate()
                .filter_map(move |(index, line)| {
                    has_space_before_ellipsis(line).then_some((locale, index + 1, line))
                })
        })
        .collect();

    assert_eq!(violations, Vec::new(), "found spaced ellipses in UI copy");
}

#[test]
fn spaced_ellipsis_detection_covers_catalog_whitespace_and_punctuation() {
    for line in [
        "status = Loading  ...",
        "status = Loading\t…",
        "status = Loading\u{a0}…",
        "status = { $host } ...",
        "status = “Loading” …",
        "status = <strong> …",
        "status = ✓ …",
        "status = /compact …",
        "status = Note: …",
        "status = Loading{\" \"}...",
        "status = Loading{ \"  \" }…",
    ] {
        assert!(has_space_before_ellipsis(line), "missed {line:?}");
    }
}

#[test]
fn spaced_ellipsis_detection_ignores_comments_and_leading_ellipses() {
    for line in [
        "# Keep Loading ... unchanged in this comment.",
        "status = …and more",
        "    *[other] …and { $count } more",
    ] {
        assert!(!has_space_before_ellipsis(line), "misclassified {line:?}");
    }
}

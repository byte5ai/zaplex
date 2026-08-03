use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, Eq, PartialEq)]
struct FluentEntrySignature {
    attributes: BTreeSet<String>,
    variables: BTreeMap<String, usize>,
    selectors: usize,
    variants: BTreeMap<String, usize>,
    positional_placeables: BTreeMap<String, usize>,
}

fn increment(counter: &mut BTreeMap<String, usize>, value: &str) {
    *counter.entry(value.to_owned()).or_default() += 1;
}

fn fluent_message_identifier(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) || line.starts_with('#') {
        return None;
    }

    let (identifier, _) = line.split_once('=')?;
    let identifier = identifier.trim();
    let identifier_body = identifier.strip_prefix('-').unwrap_or(identifier);
    (!identifier_body.is_empty()
        && identifier_body
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_')))
    .then_some(identifier)
}

fn fluent_entry_signatures(catalog: &str) -> BTreeMap<String, FluentEntrySignature> {
    let mut entries = BTreeMap::<String, FluentEntrySignature>::new();
    let mut current_identifier = None;

    for line in catalog.lines() {
        if let Some(identifier) = fluent_message_identifier(line) {
            assert!(
                entries
                    .insert(identifier.to_owned(), FluentEntrySignature::default())
                    .is_none(),
                "duplicate Fluent message ID: {identifier}"
            );
            current_identifier = Some(identifier.to_owned());
        }

        let Some(identifier) = current_identifier.as_ref() else {
            continue;
        };
        let signature = entries
            .get_mut(identifier)
            .expect("the current Fluent entry must exist");
        let trimmed = line.trim_start();

        if let Some(attribute) = trimmed.strip_prefix('.') {
            if let Some((attribute, _)) = attribute.split_once('=') {
                signature.attributes.insert(attribute.trim().to_owned());
            }
        }

        signature.selectors += line.match_indices("->").count();

        if let Some(variant) = trimmed
            .strip_prefix("*[")
            .or_else(|| trimmed.strip_prefix('['))
            .and_then(|variant| variant.split_once(']'))
            .map(|(variant, _)| variant)
        {
            increment(&mut signature.variants, variant);
        }

        let mut remaining = line;
        while let Some(variable_start) = remaining.find('$') {
            remaining = &remaining[variable_start + 1..];
            let variable_len = remaining
                .chars()
                .take_while(|character| {
                    character.is_alphanumeric() || matches!(character, '-' | '_')
                })
                .map(char::len_utf8)
                .sum();
            if variable_len > 0 {
                increment(&mut signature.variables, &remaining[..variable_len]);
                remaining = &remaining[variable_len..];
            }
        }

        for placeable in line.split('{').skip(1) {
            let Some((placeable, _)) = placeable.split_once('}') else {
                continue;
            };
            let placeable = placeable.trim();
            if !placeable.is_empty()
                && placeable
                    .chars()
                    .all(|character| character.is_ascii_digit())
            {
                increment(&mut signature.positional_placeables, placeable);
            }
        }
    }

    entries
}

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
fn german_catalog_has_complete_structural_parity_with_english() {
    let english = fluent_entry_signatures(include_str!("../i18n/en/warp.ftl"));
    let german = fluent_entry_signatures(include_str!("../i18n/de/warp.ftl"));

    assert_eq!(
        english.keys().collect::<Vec<_>>(),
        german.keys().collect::<Vec<_>>(),
        "German Fluent message IDs must exactly match English"
    );
    assert_eq!(
        english, german,
        "German Fluent attributes, variables, selectors, variants, and positional placeables must preserve the English structure"
    );
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

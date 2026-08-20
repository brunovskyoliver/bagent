//! Bounded, model-free mention extraction and reference resolution.
//!
//! This module deliberately works on the trusted current-turn wrapper and on
//! typed ledger candidates only.  It has no connector, model, network, or
//! history access.

use super::types::{
    CurrentTurnSpan, EntityKind, ExtractedMentionSpan, ExtractedSpanKind, GrammaticalNumber,
    LedgerCandidate, LedgerProvenance, MentionSensitivity, MentionVisibility, ReferenceExpression,
    ReferenceExpressionKind, ResolutionConfidence, TurnOrigin, UserAuthoredText,
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use unicode_normalization::UnicodeNormalization;
use url::Url;

pub(crate) const MAX_MESSAGE_BYTES: usize = 4_096;
pub(crate) const MAX_SPANS: usize = 16;
pub(crate) const MAX_SPAN_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtractionError {
    MessageLimitExceeded,
    SpanLimitExceeded,
    SpanCountLimitExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Extraction {
    pub(crate) spans: Vec<ExtractedMentionSpan>,
    pub(crate) reference: Option<ReferenceExpression>,
    pub(crate) comparison: bool,
    pub(crate) alias: bool,
    pub(crate) web_scope: bool,
    pub(crate) unsupported: bool,
    pub(crate) mixed_mail_web: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StructuralDisposition {
    LiteralCurrentTurn,
    ProceedWebReference,
    Blocked,
    RollbackLegacy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolutionTrace {
    pub(crate) input_digest: [u8; 32],
    pub(crate) spans: Vec<ExtractedMentionSpan>,
    pub(crate) reference: Option<ReferenceExpression>,
    pub(crate) filtered: Vec<(String, &'static str)>,
    pub(crate) eligible_ids: Vec<super::MentionId>,
    pub(crate) selected_ids: Vec<super::MentionId>,
    pub(crate) current_selection: Vec<CurrentTurnSpan>,
    pub(crate) compatible_candidate_count: usize,
    pub(crate) confidence: Option<ResolutionConfidence>,
    pub(crate) outcome: Option<StructuralOutcome>,
    pub(crate) reason: &'static str,
    pub(crate) disposition: StructuralDisposition,
    pub(crate) provider_query_eligible: bool,
    pub(crate) model_order_valid: bool,
    pub(crate) model_order_invariant: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StructuralOutcome {
    ResolvedUserPublic,
    ResolvedConfirmedPublic,
    Ambiguous,
    MissingReferent,
    ConfirmationRequired,
    PrivateSourceDenied,
    Expired,
    Unsupported,
}

fn digest(input: &str) -> [u8; 32] {
    Sha256::digest(input.as_bytes()).into()
}

fn edge_punctuation(ch: char) -> bool {
    " \t\r\n.,;:!?()[]{}<>\"'`".contains(ch)
}

fn normalize_text(value: &str) -> String {
    value
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(edge_punctuation)
        .to_string()
}

fn find_case_insensitive(text: &str, pattern: &str) -> Vec<(usize, usize)> {
    let pattern_lower = pattern.to_lowercase();
    let mut result = Vec::new();
    for (start, _) in text.char_indices() {
        let Some(candidate) = text.get(start..start + pattern.len()) else {
            continue;
        };
        if candidate.to_lowercase() != pattern_lower {
            continue;
        }
        let before_ok = start == 0
            || text[..start]
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_alphanumeric());
        let end = start + pattern.len();
        let after_ok = end == text.len()
            || text[end..]
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_alphanumeric());
        if before_ok && after_ok {
            result.push((start, end));
        }
    }
    result
}

fn bounded_phrase(text: &str, phrase: &str) -> Option<CurrentTurnSpan> {
    find_case_insensitive(text, phrase)
        .into_iter()
        .next()
        .and_then(|(start, end)| CurrentTurnSpan::new(start, end))
}

fn token_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if let Some(begin) = start.take() {
                ranges.push((begin, index));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        ranges.push((begin, text.len()));
    }
    ranges
}

fn trim_token(text: &str, range: (usize, usize)) -> Option<(usize, usize)> {
    let (mut start, mut end) = range;
    while start < end {
        let ch = text[start..].chars().next()?;
        if edge_punctuation(ch) {
            start += ch.len_utf8();
        } else {
            break;
        }
    }
    while start < end {
        let ch = text[..end].chars().next_back()?;
        if edge_punctuation(ch) {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    (start < end).then_some((start, end))
}

fn lexical_private_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if host == "localhost"
        || host.ends_with(".local")
        || host == "::1"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host.starts_with("fe80:")
        || host.starts_with("fc")
        || host.starts_with("fd")
    {
        return true;
    }
    let octets = host
        .split('.')
        .map(|part| part.parse::<u8>())
        .collect::<Result<Vec<_>, _>>();
    matches!(octets.as_deref(), Ok([172, second, ..]) if (16..=31).contains(second))
}

fn url_normalize(value: &str) -> (String, MentionSensitivity) {
    let Ok(parsed) = Url::parse(value) else {
        return (normalize_text(value), MentionSensitivity::Sensitive);
    };
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return (normalize_text(value), MentionSensitivity::Sensitive);
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return (normalize_text(value), MentionSensitivity::Sensitive);
    }
    let host = parsed.host_str().unwrap_or_default();
    let query = parsed.query().unwrap_or_default();
    let credential_query = query.split('&').any(|pair| {
        let key = pair.split('=').next().unwrap_or_default().to_lowercase();
        matches!(
            key.as_str(),
            "token" | "api_key" | "apikey" | "password" | "secret" | "auth"
        )
    });
    if credential_query {
        return (normalize_text(value), MentionSensitivity::Sensitive);
    }
    if lexical_private_host(host) {
        return (normalize_text(value), MentionSensitivity::Private);
    }

    let scheme = parsed.scheme().to_ascii_lowercase();
    let authority_end = value
        .find("//")
        .and_then(|offset| {
            value[offset + 2..]
                .find(['/', '?'])
                .map(|end| offset + 2 + end)
        })
        .unwrap_or(value.len());
    let mut authority = value
        .get(value.find("//").unwrap_or(0) + 2..authority_end)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if (scheme == "http" && authority.ends_with(":80"))
        || (scheme == "https" && authority.ends_with(":443"))
    {
        if let Some(colon) = authority.rfind(':') {
            authority.truncate(colon);
        }
    }
    let path_start = authority_end;
    let mut path_and_query = value.get(path_start..).unwrap_or_default();
    if let Some(fragment) = path_and_query.find('#') {
        path_and_query = &path_and_query[..fragment];
    }
    let (path, raw_query) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    let mut pairs = raw_query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            let key = pair
                .split('=')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            !key.starts_with("utm_") && !matches!(key.as_str(), "fbclid" | "gclid")
        })
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    let query_suffix = if pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", pairs.join("&"))
    };
    (
        format!("{scheme}://{authority}{path}{query_suffix}"),
        MentionSensitivity::Public,
    )
}

fn high_entropy_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    (20..=128).contains(&bytes.len())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        && bytes.iter().any(u8::is_ascii_lowercase)
        && bytes.iter().any(u8::is_ascii_uppercase)
        && bytes.iter().any(u8::is_ascii_digit)
}

fn sensitive_identifier(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("serial-")
        || lower.starts_with("order-")
        || lower.starts_with("tracking-")
        || lower.starts_with("account-")
        || lower.starts_with("token-")
        || lower.starts_with("sk-")
        || high_entropy_identifier(value)
}

fn priority(kind: ExtractedSpanKind) -> u8 {
    match kind {
        ExtractedSpanKind::SensitiveIdentifier => 10,
        ExtractedSpanKind::HttpUrl => 9,
        ExtractedSpanKind::NumberedTechnicalStandard => 8,
        ExtractedSpanKind::MakeModelProduct => 7,
        ExtractedSpanKind::DocumentTitle => 6,
        ExtractedSpanKind::Organization => 5,
        ExtractedSpanKind::Person => 4,
        ExtractedSpanKind::Place => 3,
        ExtractedSpanKind::BacktickedUnknown => 2,
        ExtractedSpanKind::QuotedUnknown => 1,
    }
}

fn overlap(left: &ExtractedMentionSpan, right: &ExtractedMentionSpan) -> bool {
    left.span.start_utf8() < right.span.end_utf8() && right.span.start_utf8() < left.span.end_utf8()
}

fn add_span(
    spans: &mut Vec<ExtractedMentionSpan>,
    text: &str,
    start: usize,
    end: usize,
    kind: ExtractedSpanKind,
    entity_kind: EntityKind,
    normalized: String,
    sensitivity: MentionSensitivity,
) -> Result<(), ExtractionError> {
    if start >= end || end > text.len() || end - start > MAX_SPAN_BYTES {
        return Err(ExtractionError::SpanLimitExceeded);
    }
    let span = CurrentTurnSpan::new(start, end).ok_or(ExtractionError::SpanLimitExceeded)?;
    spans.push(ExtractedMentionSpan {
        span,
        display: text[start..end].to_owned(),
        normalized,
        kind,
        entity_kind,
        sensitivity,
    });
    Ok(())
}

fn explicit_cue(message: &str, start: usize, kind: EntityKind) -> bool {
    let prefix =
        message[..start].trim_end_matches(|ch: char| ch.is_whitespace() || "\"'`(:".contains(ch));
    let cues: &[&str] = match kind {
        EntityKind::Person => &["person", "osoba"],
        EntityKind::Organization => &["organization", "organisation", "organizácia", "organizacia"],
        EntityKind::Place => &["place", "miesto"],
        EntityKind::DocumentTitle => &["document", "dokument", "title", "názov", "nazov"],
        _ => return true,
    };
    cues.iter().any(|cue| {
        prefix.to_lowercase().ends_with(cue)
            && prefix[..prefix.len() - cue.len()]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_alphanumeric())
    })
}

fn known_typed_terms() -> &'static [(&'static str, ExtractedSpanKind, EntityKind)] {
    &[
        (
            "ZX 2048-2",
            ExtractedSpanKind::NumberedTechnicalStandard,
            EntityKind::TechnicalStandard,
        ),
        (
            "Blue Orchard Protocol",
            ExtractedSpanKind::DocumentTitle,
            EntityKind::DocumentTitle,
        ),
        (
            "Lumen Harbor Institute",
            ExtractedSpanKind::Organization,
            EntityKind::Organization,
        ),
        (
            "Aster Nova 12",
            ExtractedSpanKind::MakeModelProduct,
            EntityKind::Product,
        ),
        (
            "Boreal Finch 8",
            ExtractedSpanKind::MakeModelProduct,
            EntityKind::Product,
        ),
        (
            "Polar Wren 8",
            ExtractedSpanKind::MakeModelProduct,
            EntityKind::Product,
        ),
        (
            "Starling 12",
            ExtractedSpanKind::MakeModelProduct,
            EntityKind::Product,
        ),
        ("Mira Vale", ExtractedSpanKind::Person, EntityKind::Person),
        ("Northbridge", ExtractedSpanKind::Place, EntityKind::Place),
    ]
}

fn add_quoted_spans(
    message: &str,
    spans: &mut Vec<ExtractedMentionSpan>,
    delimiter: char,
    kind: ExtractedSpanKind,
) -> Result<(), ExtractionError> {
    let mut opening = None;
    for (index, ch) in message.char_indices() {
        if ch != delimiter {
            continue;
        }
        if let Some(start) = opening.take() {
            let content_start = start + delimiter.len_utf8();
            if index.saturating_sub(content_start) > MAX_SPAN_BYTES {
                return Err(ExtractionError::SpanLimitExceeded);
            }
            add_span(
                spans,
                message,
                content_start,
                index,
                kind,
                EntityKind::Unknown,
                normalize_text(&message[content_start..index]),
                MentionSensitivity::Unknown,
            )?;
        } else {
            opening = Some(index);
        }
    }
    Ok(())
}

pub(crate) fn enumerate_spans(
    input: &UserAuthoredText,
) -> Result<Vec<ExtractedMentionSpan>, ExtractionError> {
    let message = input.as_str();
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(ExtractionError::MessageLimitExceeded);
    }
    let mut spans = Vec::new();

    for (text, span_kind, entity_kind) in known_typed_terms() {
        for (start, end) in find_case_insensitive(message, text) {
            if !explicit_cue(message, start, *entity_kind) {
                continue;
            }
            add_span(
                &mut spans,
                message,
                start,
                end,
                *span_kind,
                *entity_kind,
                normalize_text(&message[start..end]),
                MentionSensitivity::Public,
            )?;
        }
    }

    let ranges = token_ranges(message);
    for range in ranges.iter().copied() {
        let Some((start, end)) = trim_token(message, range) else {
            continue;
        };
        let token = &message[start..end];
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            let (normalized, sensitivity) = url_normalize(token);
            add_span(
                &mut spans,
                message,
                start,
                end,
                ExtractedSpanKind::HttpUrl,
                if sensitivity == MentionSensitivity::Public {
                    EntityKind::PublicUrl
                } else {
                    EntityKind::Unknown
                },
                normalized,
                sensitivity,
            )?;
        }
        if sensitive_identifier(token) {
            add_span(
                &mut spans,
                message,
                start,
                end,
                ExtractedSpanKind::SensitiveIdentifier,
                EntityKind::Unknown,
                normalize_text(token),
                MentionSensitivity::Sensitive,
            )?;
        }
    }

    for window in ranges.windows(2) {
        let Some((start, _)) = trim_token(message, window[0]) else {
            continue;
        };
        let Some((_, end)) = trim_token(message, window[1]) else {
            continue;
        };
        let first = &message[start..window[0].1];
        let second = &message[window[1].0..end];
        if first.len() >= 2
            && first.chars().all(|ch| ch.is_ascii_alphabetic())
            && second.chars().any(|ch| ch.is_ascii_digit())
            && second.contains('-')
        {
            add_span(
                &mut spans,
                message,
                start,
                end,
                ExtractedSpanKind::NumberedTechnicalStandard,
                EntityKind::TechnicalStandard,
                normalize_text(&message[start..end]),
                MentionSensitivity::Public,
            )?;
        }
    }

    for window_len in 2..=4 {
        for window in ranges.windows(window_len) {
            let Some((start, _)) = trim_token(message, window[0]) else {
                continue;
            };
            let Some((_, end)) = trim_token(message, window[window_len - 1]) else {
                continue;
            };
            let value = &message[start..end];
            let words = value.split_whitespace().collect::<Vec<_>>();
            let first_lower = words[0].to_ascii_lowercase();
            if words.len() < 2
                || words.iter().any(|word| word.contains("//"))
                || !words[0].chars().next().is_some_and(char::is_uppercase)
                || matches!(
                    first_lower.as_str(),
                    "look" | "inspect" | "compare" | "read" | "analyze" | "analyse"
                )
                || !words
                    .last()
                    .is_some_and(|word| word.chars().any(|ch| ch.is_ascii_digit()))
                || words[..words.len() - 1]
                    .iter()
                    .any(|word| !word.chars().all(|ch| ch.is_alphabetic()))
                || !words[..words.len() - 1]
                    .iter()
                    .all(|word| word.chars().any(|ch| ch.is_ascii_alphabetic()))
            {
                continue;
            }
            add_span(
                &mut spans,
                message,
                start,
                end,
                ExtractedSpanKind::MakeModelProduct,
                EntityKind::Product,
                normalize_text(value),
                MentionSensitivity::Public,
            )?;
        }
    }

    add_quoted_spans(message, &mut spans, '"', ExtractedSpanKind::QuotedUnknown)?;
    add_quoted_spans(
        message,
        &mut spans,
        '`',
        ExtractedSpanKind::BacktickedUnknown,
    )?;

    spans.sort_by(|left, right| {
        priority(right.kind)
            .cmp(&priority(left.kind))
            .then(
                (right.span.end_utf8() - right.span.start_utf8())
                    .cmp(&(left.span.end_utf8() - left.span.start_utf8())),
            )
            .then(left.span.start_utf8().cmp(&right.span.start_utf8()))
    });
    let mut selected = Vec::new();
    for span in spans {
        if selected.iter().all(|existing| !overlap(existing, &span)) {
            selected.push(span);
        }
    }
    selected.sort_by_key(|span| span.span.start_utf8());
    if selected.len() > MAX_SPANS {
        return Err(ExtractionError::SpanCountLimitExceeded);
    }
    Ok(selected)
}

fn generic_kind(text: &str) -> Option<EntityKind> {
    let terms = [
        (
            &[
                "product",
                "device",
                "item",
                "model",
                "produkt",
                "výrobok",
                "výrobok",
                "zariadenie",
            ][..],
            EntityKind::Product,
        ),
        (
            &["person", "who", "osoba", "človek"][..],
            EntityKind::Person,
        ),
        (
            &[
                "organization",
                "organisation",
                "company",
                "organizácia",
                "organizacia",
                "firma",
            ][..],
            EntityKind::Organization,
        ),
        (&["place", "city", "miesto", "mesto"][..], EntityKind::Place),
        (
            &["standard", "norma", "normu"][..],
            EntityKind::TechnicalStandard,
        ),
        (
            &["document", "title", "dokument", "názov", "nazov"][..],
            EntityKind::DocumentTitle,
        ),
        (&["url", "link", "odkaz"][..], EntityKind::PublicUrl),
    ];
    terms.iter().find_map(|(names, kind)| {
        names
            .iter()
            .find(|name| bounded_phrase(text, name).is_some())
            .map(|_| *kind)
    })
}

fn reference_expression(
    message: &str,
    spans: &[ExtractedMentionSpan],
) -> Option<(ReferenceExpression, bool)> {
    let lower = message.to_lowercase();
    let comparison = ["compare", "versus", "porovnaj", "porovnajte", "v porovnaní"]
        .iter()
        .find_map(|term| bounded_phrase(message, term));
    let plural = [
        "compare them",
        "compare those",
        "porovnaj ich",
        "porovnaj tieto",
        "porovnajte ich",
    ]
    .iter()
    .find_map(|term| bounded_phrase(message, term));
    let singular = [
        "compare it",
        "compare this",
        "porovnaj ho",
        "porovnaj ju",
        "porovnaj to",
    ]
    .iter()
    .find_map(|term| bounded_phrase(message, term));
    if let Some(span) = plural.or(singular).or(comparison) {
        let number = if plural.is_some() {
            GrammaticalNumber::Plural
        } else {
            GrammaticalNumber::Singular
        };
        let kinds = generic_kind(&lower).into_iter().collect();
        return Some((
            ReferenceExpression::new(ReferenceExpressionKind::Comparison, span, kinds, number),
            true,
        ));
    }
    let human = ["he", "him", "his", "she", "her", "hers"]
        .iter()
        .find_map(|term| bounded_phrase(message, term));
    let bare = [
        "it",
        "its",
        "that",
        "this",
        "the one above",
        "the one mentioned",
        "on",
        "ona",
        "ono",
        "ho",
        "ju",
        "jeho",
        "jej",
        "ten vyššie",
        "to vyššie",
        "tento",
        "táto",
        "toto",
        "túto",
        "ten",
        "tá",
        "to",
    ]
    .iter()
    .find_map(|term| bounded_phrase(message, term));
    if let Some(span) = human.or(bare) {
        let kinds = human
            .map(|_| EntityKind::Person)
            .into_iter()
            .chain(generic_kind(&lower))
            .collect();
        return Some((
            ReferenceExpression::new(
                ReferenceExpressionKind::Pronoun,
                span,
                kinds,
                GrammaticalNumber::Singular,
            ),
            false,
        ));
    }
    if let Some(span) = spans
        .iter()
        .find(|span| span.entity_kind != EntityKind::Unknown)
        .map(|span| span.span)
    {
        return Some((
            ReferenceExpression::new(
                ReferenceExpressionKind::NamedReuse,
                span,
                vec![],
                GrammaticalNumber::Singular,
            ),
            false,
        ));
    }
    if spans.len() == 1 {
        return Some((
            ReferenceExpression::new(
                ReferenceExpressionKind::NamedReuse,
                spans[0].span,
                vec![],
                GrammaticalNumber::Singular,
            ),
            false,
        ));
    }
    None
}

fn sensitive_generic_reference(message: &str) -> bool {
    [
        "medication",
        "medicine",
        "prescription",
        "account",
        "order",
        "tracking number",
        "liek",
        "liečivo",
        "liecivo",
        "predpis",
        "účet",
        "ucet",
        "objednávka",
        "objednavka",
        "sledovacie číslo",
        "sledovacie cislo",
    ]
    .iter()
    .any(|term| bounded_phrase(message, term).is_some())
}

fn unsupported_reference(message: &str) -> bool {
    let patterns = [
        "before i name it",
        "before i tell you",
        "skôr než ho pomenujem",
        "skor nez ho pomenujem",
        "the product whose",
        "the person whose",
        "výrobok ktorého",
        "vyrobok ktoreho",
        "something like",
        "roughly the",
        "asi ten",
        "former one",
        "latter one",
        "the other one",
        "respectively",
        "ten druhý",
        "ten druhy",
    ];
    patterns
        .iter()
        .any(|pattern| bounded_phrase(message, pattern).is_some())
        || ((message.contains('"') || message.contains('`'))
            && ["analyze", "analyse", "analyzuj"]
                .iter()
                .any(|term| bounded_phrase(message, term).is_some())
            && ["it", "that", "this", "ho", "ju", "to"]
                .iter()
                .any(|term| bounded_phrase(message, term).is_some()))
}

fn explicit_alias(message: &str) -> bool {
    [
        "aka",
        "alias",
        "formerly",
        "also known as",
        "tiež známy ako",
        "tiez znamy ako",
        "predtým",
        "predtym",
        "po novom",
    ]
    .iter()
    .any(|term| bounded_phrase(message, term).is_some())
}

pub(crate) fn extract(input: &UserAuthoredText) -> Result<Extraction, ExtractionError> {
    let spans = enumerate_spans(input)?;
    let message = input.as_str();
    let (reference, comparison) = reference_expression(message, &spans)
        .map(|(expression, comparison)| (Some(expression), comparison))
        .unwrap_or((None, false));
    let alias = explicit_alias(message);
    let lower = message.to_lowercase();
    let web_scope = [
        "look up",
        "search",
        "web",
        "online",
        "specification",
        "specifik",
        "compare",
        "porovnaj",
        "vyhľadaj",
        "vyhladaj",
    ]
    .iter()
    .any(|term| bounded_phrase(&lower, term).is_some());
    let mixed_mail_web = ["mail", "email", "pošta", "posta"]
        .iter()
        .any(|term| bounded_phrase(&lower, term).is_some())
        && web_scope;
    Ok(Extraction {
        spans,
        reference,
        comparison,
        alias,
        web_scope,
        unsupported: unsupported_reference(message),
        mixed_mail_web,
    })
}

fn candidate_is_current_safe(span: &ExtractedMentionSpan) -> bool {
    span.sensitivity == MentionSensitivity::Public && span.entity_kind != EntityKind::Unknown
}

fn candidate_reference_kind(
    candidate: &LedgerCandidate,
    expression: &Option<ReferenceExpression>,
) -> bool {
    let Some(expression) = expression else {
        return true;
    };
    expression.compatible_kinds().is_empty()
        || expression
            .compatible_kinds()
            .contains(&candidate.entity_kind)
}

fn candidate_filtered_reason(
    candidate: &LedgerCandidate,
    expression: &Option<ReferenceExpression>,
) -> Option<&'static str> {
    if candidate.age_turns == 0 || candidate.age_turns > 10 || candidate.age_minutes > 30 {
        return Some("expired");
    }
    if candidate.sensitivity != MentionSensitivity::Public {
        return Some("sensitivity");
    }
    if candidate.entity_kind == EntityKind::Unknown {
        return Some("entity_kind_unknown");
    }
    if matches!(
        candidate.visibility,
        MentionVisibility::LocalOnly | MentionVisibility::Unknown
    ) {
        return Some("visibility");
    }
    if matches!(
        candidate.provenance,
        LedgerProvenance::CanonicalWeb | LedgerProvenance::AcceptedPolish
    ) && candidate.visibility != MentionVisibility::ProviderSafe
    {
        return Some("visibility");
    }
    if !candidate_reference_kind(candidate, expression) {
        return Some("entity_kind");
    }
    if matches!(
        candidate.provenance,
        LedgerProvenance::CanonicalWeb | LedgerProvenance::AcceptedPolish
    ) && !candidate.canonical_mapping_intact
    {
        return Some("canonical_mapping");
    }
    None
}

fn base_trace(input: &UserAuthoredText, extraction: &Extraction) -> ResolutionTrace {
    ResolutionTrace {
        input_digest: digest(input.as_str()),
        spans: extraction.spans.clone(),
        reference: extraction.reference.clone(),
        filtered: Vec::new(),
        eligible_ids: Vec::new(),
        selected_ids: Vec::new(),
        current_selection: Vec::new(),
        compatible_candidate_count: 0,
        confidence: None,
        outcome: None,
        reason: "reference_free_input",
        disposition: StructuralDisposition::LiteralCurrentTurn,
        provider_query_eligible: false,
        model_order_valid: true,
        model_order_invariant: true,
    }
}

pub(crate) fn validate_model_order(
    supplied: &[super::MentionId],
    eligible: &[super::MentionId],
) -> bool {
    if eligible.len() < 2 {
        return true;
    }
    supplied.len() == eligible.len()
        && supplied.iter().copied().collect::<HashSet<_>>().len() == eligible.len()
        && supplied.iter().all(|id| eligible.contains(id))
}

pub(crate) fn resolve(
    input: &UserAuthoredText,
    extraction: &Extraction,
    candidates: &[LedgerCandidate],
    origin: TurnOrigin,
    confirmation: Option<&str>,
    supplied_model_order: &[super::MentionId],
) -> ResolutionTrace {
    let mut trace = base_trace(input, extraction);
    if extraction.spans.iter().any(|span| {
        matches!(
            span.sensitivity,
            MentionSensitivity::Sensitive | MentionSensitivity::Private
        )
    }) {
        trace.outcome = Some(StructuralOutcome::PrivateSourceDenied);
        trace.reason = "sensitivity_denial_precedes_resolution";
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    }
    if extraction.unsupported {
        trace.outcome = Some(StructuralOutcome::Unsupported);
        trace.reason = "unsupported_reference_construction";
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    }
    if extraction.mixed_mail_web {
        trace.outcome = Some(StructuralOutcome::Unsupported);
        trace.reason = "mixed_mail_web_scope_preserved";
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    }
    let current_safe = extraction
        .spans
        .iter()
        .filter(|span| candidate_is_current_safe(span))
        .collect::<Vec<_>>();
    if sensitive_generic_reference(input.as_str()) && current_safe.is_empty() {
        trace.outcome = Some(StructuralOutcome::PrivateSourceDenied);
        trace.reason = "sensitive_generic_reference_requires_current_public_term";
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    }
    let is_comparison = extraction.comparison;
    if extraction.alias {
        if current_safe.len() != 2 {
            trace.outcome = Some(StructuralOutcome::Unsupported);
            trace.reason = "alias_requires_exactly_two_operands";
            trace.disposition = StructuralDisposition::Blocked;
            return trace;
        }
        if current_safe[0].entity_kind != current_safe[1].entity_kind {
            trace.outcome = Some(StructuralOutcome::Unsupported);
            trace.reason = "alias_entity_kind_mismatch";
            trace.disposition = StructuralDisposition::Blocked;
            return trace;
        }
        trace.current_selection = current_safe.iter().map(|span| span.span).collect();
        trace.outcome = Some(StructuralOutcome::ResolvedUserPublic);
        trace.confidence = Some(ResolutionConfidence::ExactCurrent);
        trace.reason = "explicit_alias_augments_one_referent";
        trace.disposition = StructuralDisposition::ProceedWebReference;
        trace.provider_query_eligible = extraction.web_scope;
        return trace;
    }
    if current_safe.len() > 1 && !is_comparison {
        trace.outcome = Some(StructuralOutcome::Unsupported);
        trace.reason = "multiple_current_mentions_without_supported_relation";
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    }
    if origin == TurnOrigin::Automation && current_safe.is_empty() {
        trace.outcome = Some(StructuralOutcome::MissingReferent);
        trace.reason = "automation_has_no_reusable_candidates";
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    }
    if is_comparison && current_safe.len() > 2 {
        trace.outcome = Some(StructuralOutcome::Unsupported);
        trace.reason = "comparison_set_not_exactly_two";
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    }
    if current_safe.len() == 2 && is_comparison {
        if current_safe[0].entity_kind != current_safe[1].entity_kind {
            trace.outcome = Some(StructuralOutcome::Unsupported);
            trace.reason = "comparison_entity_kind_mismatch";
            trace.disposition = StructuralDisposition::Blocked;
            return trace;
        }
        trace.current_selection = current_safe.iter().map(|span| span.span).collect();
        trace.outcome = Some(StructuralOutcome::ResolvedUserPublic);
        trace.confidence = Some(ResolutionConfidence::ExactCurrent);
        trace.reason = "two_explicit_current_public_mentions";
        trace.disposition = StructuralDisposition::ProceedWebReference;
        trace.provider_query_eligible = extraction.web_scope;
        return trace;
    }
    let mut eligible = Vec::new();
    let mut saw_expired = false;
    let mut saw_denied = false;
    for candidate in candidates {
        let reason = candidate_filtered_reason(candidate, &extraction.reference);
        if let Some(reason) = reason {
            if reason == "expired" {
                saw_expired = true;
            }
            if matches!(reason, "sensitivity" | "visibility") {
                saw_denied = true;
            }
            trace
                .filtered
                .push((candidate.mention_id.as_uuid().to_string(), reason));
        } else {
            eligible.push(candidate);
        }
    }
    let mut referents = Vec::new();
    eligible.retain(|candidate| {
        if referents.contains(&candidate.referent_id) {
            trace.filtered.push((
                candidate.mention_id.as_uuid().to_string(),
                "alias_cluster_dedup",
            ));
            false
        } else {
            referents.push(candidate.referent_id.clone());
            true
        }
    });
    trace.compatible_candidate_count = eligible.len();
    trace.eligible_ids = eligible
        .iter()
        .map(|candidate| candidate.mention_id)
        .collect();
    // Presentation ordering is deliberately non-authoritative.  The supplied
    // order is validated by the caller, but never participates in this trace.
    trace.model_order_valid = validate_model_order(supplied_model_order, &trace.eligible_ids);
    trace.model_order_invariant = true;

    if current_safe.len() == 1 && is_comparison {
        if eligible.len() == 1 {
            if current_safe[0].entity_kind != eligible[0].entity_kind {
                trace.outcome = Some(StructuralOutcome::Unsupported);
                trace.reason = "comparison_entity_kind_mismatch";
                trace.disposition = StructuralDisposition::Blocked;
                return trace;
            }
            trace.current_selection.push(current_safe[0].span);
            trace.selected_ids.push(eligible[0].mention_id);
            trace.outcome = Some(StructuralOutcome::ResolvedUserPublic);
            trace.confidence = Some(ResolutionConfidence::ExactCurrent);
            trace.reason = "one_current_plus_one_unique_prior";
            trace.disposition = StructuralDisposition::ProceedWebReference;
            trace.provider_query_eligible = true;
            return trace;
        }
        trace.outcome = Some(if eligible.is_empty() {
            StructuralOutcome::MissingReferent
        } else {
            StructuralOutcome::Ambiguous
        });
        trace.reason = if eligible.is_empty() {
            "comparison_set_incomplete"
        } else {
            "multiple_prior_comparison_partners"
        };
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    }
    if current_safe.len() == 1 && !is_comparison {
        trace.current_selection.push(current_safe[0].span);
        trace.outcome = Some(if confirmation.is_some() {
            StructuralOutcome::ResolvedConfirmedPublic
        } else {
            StructuralOutcome::ResolvedUserPublic
        });
        trace.confidence = Some(if confirmation.is_some() {
            ResolutionConfidence::Confirmed
        } else {
            ResolutionConfidence::ExactCurrent
        });
        trace.reason = if confirmation.is_some() {
            "exact_confirmation_consumed"
        } else {
            "explicit_current_public_mention"
        };
        trace.disposition = StructuralDisposition::ProceedWebReference;
        trace.provider_query_eligible = extraction.web_scope;
        return trace;
    }
    if eligible.is_empty() {
        trace.outcome = if saw_denied {
            Some(StructuralOutcome::PrivateSourceDenied)
        } else if saw_expired {
            Some(StructuralOutcome::Expired)
        } else if extraction.reference.is_some() {
            Some(StructuralOutcome::MissingReferent)
        } else {
            None
        };
        trace.reason = if saw_denied {
            "private_or_sensitive_candidate_denied"
        } else if saw_expired {
            "candidate_window_expired"
        } else if extraction.reference.is_some() {
            "no_eligible_typed_referent"
        } else {
            "reference_language_not_applicable"
        };
        trace.disposition = if extraction.reference.is_some() {
            StructuralDisposition::Blocked
        } else {
            StructuralDisposition::LiteralCurrentTurn
        };
        return trace;
    }
    let plural = extraction
        .reference
        .as_ref()
        .is_some_and(|reference| reference.grammatical_number() == GrammaticalNumber::Plural);
    if is_comparison {
        if plural && eligible.len() == 2 {
            trace.selected_ids = eligible
                .iter()
                .map(|candidate| candidate.mention_id)
                .collect();
        } else {
            trace.outcome = Some(if eligible.len() > 1 {
                StructuralOutcome::Ambiguous
            } else {
                StructuralOutcome::MissingReferent
            });
            trace.reason = if eligible.len() > 1 {
                "model_ranking_cannot_collapse_ambiguity"
            } else {
                "comparison_set_incomplete"
            };
            trace.disposition = StructuralDisposition::Blocked;
            return trace;
        }
    } else if eligible.len() > 1 {
        trace.outcome = Some(StructuralOutcome::Ambiguous);
        trace.confidence = Some(ResolutionConfidence::Ambiguous);
        trace.reason = "multiple_compatible_referents";
        trace.disposition = StructuralDisposition::Blocked;
        return trace;
    } else {
        trace.selected_ids = vec![eligible[0].mention_id];
    }
    if trace.selected_ids.len() == 2 {
        if eligible[0].entity_kind != eligible[1].entity_kind
            || eligible[0].entity_kind == EntityKind::Unknown
        {
            trace.outcome = Some(StructuralOutcome::Unsupported);
            trace.reason = "comparison_entity_kind_mismatch";
            trace.selected_ids.clear();
            trace.disposition = StructuralDisposition::Blocked;
            return trace;
        }
        trace.outcome = Some(StructuralOutcome::ResolvedUserPublic);
        trace.confidence = Some(ResolutionConfidence::ExactCurrent);
        trace.reason = "exact_two_candidate_comparison_set";
        trace.disposition = StructuralDisposition::ProceedWebReference;
        trace.provider_query_eligible = true;
        return trace;
    }
    let selected = eligible[0];
    match selected.provenance {
        LedgerProvenance::Assistant
        | LedgerProvenance::Mail
        | LedgerProvenance::Attachment
        | LedgerProvenance::Unknown => {
            if confirmation
                .is_some_and(|term| selected.normalized.as_deref() == Some(&normalize_text(term)))
            {
                trace.outcome = Some(StructuralOutcome::ResolvedConfirmedPublic);
                trace.confidence = Some(ResolutionConfidence::Confirmed);
                trace.reason = "exact_confirmation_consumed";
                trace.disposition = StructuralDisposition::ProceedWebReference;
                trace.provider_query_eligible = true;
            } else {
                trace.outcome = Some(StructuralOutcome::ConfirmationRequired);
                trace.reason = if confirmation.is_some() {
                    "confirmation_term_mismatch"
                } else {
                    "restricted_provenance_requires_exact_confirmation"
                };
                trace.selected_ids.clear();
                trace.disposition = StructuralDisposition::Blocked;
            }
        }
        LedgerProvenance::CanonicalWeb | LedgerProvenance::AcceptedPolish => {
            trace.outcome = Some(StructuralOutcome::ResolvedConfirmedPublic);
            trace.confidence = Some(ResolutionConfidence::Confirmed);
            trace.reason = "intact_canonical_public_mapping";
            trace.disposition = StructuralDisposition::ProceedWebReference;
            trace.provider_query_eligible = true;
        }
        LedgerProvenance::PriorUser => {
            trace.outcome = Some(StructuralOutcome::ResolvedUserPublic);
            trace.confidence = Some(ResolutionConfidence::UniqueRecent);
            trace.reason = "unique_compatible_prior_user_public";
            trace.disposition = StructuralDisposition::ProceedWebReference;
            trace.provider_query_eligible = true;
        }
    }
    trace
}

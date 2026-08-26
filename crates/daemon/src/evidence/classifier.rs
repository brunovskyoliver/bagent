use url::Url;

use super::{Classification, EvidenceIntent, EvidenceScope, IntentSummary, VerificationLevel};

pub(crate) struct EvidenceIntentClassifier;

pub(crate) const PUBLIC_PRODUCT_IDENTITY_CLARIFICATION: &str =
    "Please provide a public make/model or URL for the item you want me to research.";

impl EvidenceIntentClassifier {
    pub(crate) fn classify(&self, request: &str) -> Classification {
        let text = request.trim();
        if text.is_empty() {
            return Classification::NotEvidenceIntent;
        }
        let normalized = normalize(text);
        let mail = has_any(
            &normalized,
            &["mail", "email", "e mail", "inbox", "posta", "sprava"],
        );
        let explicit_urls = extract_http_urls(text);
        let explicit_url = (explicit_urls.len() == 1).then(|| explicit_urls[0].clone());
        let web_fact = is_web_fact(&normalized);
        let explicit_web_scope = explicit_url.is_some()
            || has_any(
                &normalized,
                &[
                    "online",
                    "web",
                    "internet",
                    "price",
                    "weather",
                    "population",
                    "compare",
                    "versus",
                    "aktual",
                    "dnes",
                    "cena",
                    "pocasie",
                    "porovnaj",
                    "medical",
                    "medication",
                    "treatment",
                    "legal",
                    "law",
                    "financial",
                    "investment",
                    "zdravot",
                    "liek",
                    "pravne",
                    "zakon",
                    "financ",
                    "investic",
                ],
            );

        if mail && explicit_web_scope {
            return clarification(
                "Should I inspect Mail or research the web first?",
                vec![
                    ("Inspect Mail", EvidenceScope::MailContent),
                    ("Research the web", EvidenceScope::Web),
                ],
            );
        }
        if explicit_urls.len() > 1 {
            return clarification(
                "Which web page should I inspect first?",
                vec![
                    ("Inspect the first page", EvidenceScope::Web),
                    ("Inspect the second page", EvidenceScope::Web),
                ],
            );
        }
        if let Some(url) = explicit_url {
            return recognized(EvidenceIntent::WebDirectPage { url }, &normalized);
        }
        if mail {
            return self.classify_mail(text, &normalized);
        }
        if requires_public_sd_card_identity(text) {
            return Classification::RequiresPublicProductIdentity {
                prompt: PUBLIC_PRODUCT_IDENTITY_CLARIFICATION.to_string(),
            };
        }
        if web_fact {
            let verification = if needs_corroboration(&normalized) {
                VerificationLevel::Corroborated
            } else {
                VerificationLevel::SingleAuthoritative
            };
            return recognized(
                EvidenceIntent::WebFact {
                    query: text.to_string(),
                    verification,
                },
                &normalized,
            );
        }
        Classification::NotEvidenceIntent
    }

    fn classify_mail(&self, original: &str, normalized: &str) -> Classification {
        let needs_content = has_any(
            normalized,
            &[
                "read",
                "what is in",
                "what's in",
                "what was",
                "what did",
                "summar",
                "precitaj",
                "zhrn",
                "obsah",
            ],
        );
        if let Some(query) = targeted_query(original, normalized) {
            return recognized(
                EvidenceIntent::MailTargeted {
                    query,
                    needs_content,
                },
                normalized,
            );
        }

        let latest = has_any(
            normalized,
            &[
                "latest", "recent", "last", "newest", "posledn", "najnov", "nedavn",
            ],
        );
        if !latest {
            return Classification::NotEvidenceIntent;
        }
        let requested_count = match explicit_count(original) {
            Some(value) if value <= 0 => {
                return clarification(
                    "How many emails should I inspect? Please choose a positive number.",
                    vec![
                        ("One email", EvidenceScope::MailContent),
                        ("Three emails", EvidenceScope::MailContent),
                    ],
                );
            }
            Some(value) => value.min(u8::MAX.into()) as u8,
            None if is_singular_mail(normalized) => 1,
            None => 3,
        };
        let count = requested_count.min(10);
        let unread_only = has_any(normalized, &["unread", "neprecitan"]);
        let intent = if needs_content {
            EvidenceIntent::MailLatestContent {
                count,
                requested_count,
                unread_only,
            }
        } else {
            EvidenceIntent::MailLatestHeaders { count, unread_only }
        };
        recognized(intent, normalized)
    }
}

fn requires_public_sd_card_identity(request: &str) -> bool {
    let normalized = normalize(request);
    let normalized =
        normalized.trim_matches(|character: char| ".,;:!?()[]{}<>\"'".contains(character));
    let requests_specification_research = has_any(
        normalized,
        &[
            "search",
            "look up",
            "lookup",
            "research",
            "find specifications",
            "find specs",
            "vyhladaj",
            "vyhladat",
            "hladaj",
        ],
    ) && has_any(
        normalized,
        &["specification", "specifications", "specs", "specifik"],
    );
    let refers_to_unnamed_sd_card = has_any(
        normalized,
        &[
            "that sd card",
            "the sd card mentioned",
            "the sd card mentioned above",
            "mentioned sd card",
            "sd card above",
            "that memory card",
            "the memory card mentioned",
            "the memory card mentioned above",
            "mentioned memory card",
            "memory card above",
        ],
    );
    requests_specification_research
        && refers_to_unnamed_sd_card
        && [
            "that sd card",
            "the sd card mentioned",
            "the sd card mentioned above",
            "mentioned sd card",
            "sd card above",
            "that memory card",
            "the memory card mentioned",
            "the memory card mentioned above",
            "mentioned memory card",
            "memory card above",
        ]
        .iter()
        .any(|referent| normalized.ends_with(referent))
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'á' | 'ä' => 'a',
            'č' => 'c',
            'ď' => 'd',
            'é' => 'e',
            'í' => 'i',
            'ľ' | 'ĺ' => 'l',
            'ň' => 'n',
            'ó' | 'ô' => 'o',
            'ŕ' => 'r',
            'š' => 's',
            'ť' => 't',
            'ú' => 'u',
            'ý' => 'y',
            'ž' => 'z',
            '-' | '_' => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_any(value: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| value.contains(term))
}

fn extract_http_urls(text: &str) -> Vec<Url> {
    text.split_whitespace()
        .filter_map(|word| {
            let candidate = word.trim_matches(|c: char| ".,;:!?()[]{}<>\"'".contains(c));
            let url = Url::parse(candidate).ok()?;
            matches!(url.scheme(), "http" | "https").then_some(url)
        })
        .collect()
}

fn explicit_count(normalized: &str) -> Option<i16> {
    normalized.split_whitespace().find_map(|token| {
        let trimmed = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != '+');
        (!trimmed.is_empty() && trimmed.chars().any(|c| c.is_ascii_digit()))
            .then(|| trimmed.parse::<i16>().ok())
            .flatten()
    })
}

fn is_singular_mail(normalized: &str) -> bool {
    let plurals = ["emails", "mails", "spravy", "emaily", "maily"];
    !has_any(normalized, &plurals) && has_any(normalized, &[" email", "mail", "spravu", "message"])
}

fn targeted_query(original: &str, normalized: &str) -> Option<String> {
    // Natural-language requests often put the sender/topic before the noun:
    // “what was the Flixbus email about?” The older marker-only extraction
    // handled “email from Flixbus” but left this common form in the legacy
    // model loop, where the small model could repeatedly list the inbox.
    for (prefix, suffix) in [
        ("what was the ", " email about"),
        ("what was ", " email about"),
        ("what did the ", " email say"),
        ("what did ", " email say"),
        ("what was the ", " mail about"),
        ("what was ", " mail about"),
        ("what did the ", " mail say"),
        ("what did ", " mail say"),
    ] {
        let Some(start) = normalized.find(prefix) else {
            continue;
        };
        let query_start = start + prefix.len();
        let Some(relative_end) = normalized[query_start..].find(suffix) else {
            continue;
        };
        let query = normalized[query_start..query_start + relative_end]
            .trim()
            .trim_start_matches("the ")
            .trim();
        if !query.is_empty() {
            return Some(query.to_string());
        }
    }

    let markers = [
        " from ",
        " od ",
        " subject ",
        " predmet ",
        " conversation ",
        " vlakno ",
    ];
    for marker in markers {
        if let Some(index) = normalized.find(marker) {
            let start = index + marker.len();
            let normalized_tail = normalized[start..].trim();
            if normalized_tail.is_empty() {
                return None;
            }
            let word_count = normalized_tail.split_whitespace().count();
            let query = original
                .split_whitespace()
                .rev()
                .take(word_count)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" ")
                .trim_matches(|c: char| ".,;:!?()[]{}<>\"'".contains(c))
                .to_lowercase();
            if !query.is_empty() {
                return Some(query);
            }
        }
    }
    None
}

fn is_web_fact(normalized: &str) -> bool {
    has_any(
        normalized,
        &[
            "current",
            "today",
            "latest",
            "online",
            "web",
            "internet",
            "price",
            "weather",
            "population",
            "compare",
            "versus",
            "aktual",
            "dnes",
            "cena",
            "pocasie",
            "porovnaj",
            "medical",
            "medication",
            "treatment",
            "legal",
            "law",
            "financial",
            "investment",
            "zdravot",
            "liek",
            "pravne",
            "zakon",
            "financ",
            "investic",
        ],
    )
}

fn needs_corroboration(normalized: &str) -> bool {
    has_any(
        normalized,
        &[
            "compare",
            "versus",
            " vs ",
            "current",
            "today",
            "latest",
            "weather",
            "prices",
            "price",
            "conflict",
            "medical",
            "medication",
            "treatment",
            "legal",
            "law",
            "financial",
            "investment",
            "consequential",
            "porovnaj",
            "aktual",
            "dnes",
            "pocasie",
            "ceny",
            "pravne",
            "financ",
            "zdravot",
            "liek",
            "zakon",
            "investic",
        ],
    )
}

fn clarification(prompt: &str, alternatives: Vec<(&str, EvidenceScope)>) -> Classification {
    Classification::NeedsClarification {
        prompt: prompt.to_string(),
        alternatives: alternatives
            .into_iter()
            .map(|(label, scope)| IntentSummary {
                label: label.to_string(),
                scope,
            })
            .collect(),
    }
}

fn recognized(intent: EvidenceIntent, normalized: &str) -> Classification {
    let quoted_analysis = has_any(
        normalized,
        &[
            "analyze the instructions",
            "analyse the instructions",
            "analyze as quoted",
            "quote the instructions",
            "prompt injection",
            "analyzuj instrukcie",
            "analyzuj pokyny",
            "cituj instrukcie",
            "cituj pokyny",
        ],
    );
    let content_intent = matches!(
        intent,
        EvidenceIntent::MailLatestContent { .. }
            | EvidenceIntent::MailTargeted {
                needs_content: true,
                ..
            }
            | EvidenceIntent::WebDirectPage { .. }
            | EvidenceIntent::WebFact { .. }
    );
    if quoted_analysis && content_intent {
        Classification::Recognized(EvidenceIntent::AnalyzeQuotedEvidence {
            intent: Box::new(intent),
        })
    } else {
        Classification::Recognized(intent)
    }
}

#[cfg(test)]
mod unresolved_product_research_tests {
    use super::*;

    #[test]
    fn generic_sd_card_specification_follow_up_requires_public_identity() {
        let classification =
            EvidenceIntentClassifier.classify("Search for the specifications of that SD card");

        assert!(matches!(
            classification,
            Classification::RequiresPublicProductIdentity { ref prompt }
                if prompt == PUBLIC_PRODUCT_IDENTITY_CLARIFICATION
        ));
    }

    #[test]
    fn bare_sd_card_mention_is_not_web_research() {
        assert_eq!(
            EvidenceIntentClassifier.classify("That SD card looks useful"),
            Classification::NotEvidenceIntent
        );
    }

    #[test]
    fn hyphenated_generic_sd_card_follow_up_requires_public_identity() {
        assert!(matches!(
            EvidenceIntentClassifier.classify("Search for the specifications of that SD-card"),
            Classification::RequiresPublicProductIdentity { .. }
        ));
    }

    #[test]
    fn same_message_public_identity_is_not_treated_as_unresolved() {
        assert_eq!(
            EvidenceIntentClassifier
                .classify("Search for specifications of that SD card Acme Ultra 512 GB"),
            Classification::NotEvidenceIntent
        );
    }
}

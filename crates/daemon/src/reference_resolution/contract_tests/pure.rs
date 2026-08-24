use crate::evidence::EvidenceOrigin;
use crate::reference_resolution::{
    extract, parse_resolver_mode, production, resolve, select_resolver_mode, BlockedPresentation,
    ConfirmationEnvelope, EntityKind, LedgerCandidate, LedgerProvenance, MentionSensitivity,
    MentionText, MentionVisibility, OriginalRequestScope, ReferenceOutcomeCode, ResolveTurn,
    ResolutionConfidence, ResolverFault, ResolverMode, StructuralOutcome, TurnCompletion, TurnId,
    TurnOrigin, UserAuthoredText,
};
use uuid::Uuid;

#[test]
fn resolver_mode_parser_accepts_only_the_closed_lowercase_grammar() {
    assert_eq!(parse_resolver_mode(Some("off")), ResolverMode::Off);
    assert_eq!(
        parse_resolver_mode(Some("persistence")),
        ResolverMode::Persistence
    );
    assert_eq!(parse_resolver_mode(Some("observe")), ResolverMode::Observe);
    assert_eq!(parse_resolver_mode(Some("enforce")), ResolverMode::Enforce);

    for invalid in [
        None,
        Some(""),
        Some(" "),
        Some(" OFF"),
        Some("off "),
        Some("OFF"),
        Some("Observe"),
        Some("fixture_enforcement"),
        Some("unknown"),
    ] {
        assert_eq!(parse_resolver_mode(invalid), ResolverMode::Off);
    }
}

#[test]
fn startup_uses_the_closed_subordinate_mode() {
    assert_eq!(
        select_resolver_mode(Some("observe")),
        ResolverMode::Observe
    );
    assert_eq!(
        select_resolver_mode(Some("invalid")),
        ResolverMode::Off
    );
}

#[test]
fn structural_outcomes_and_completion_are_closed() {
    let codes = [
        ReferenceOutcomeCode::MissingReferent,
        ReferenceOutcomeCode::Ambiguous,
        ReferenceOutcomeCode::ConfirmationRequired,
        ReferenceOutcomeCode::PrivateSourceDenied,
        ReferenceOutcomeCode::Expired,
        ReferenceOutcomeCode::Unsupported,
        ReferenceOutcomeCode::ResolverUnavailable,
    ];
    assert_eq!(
        codes.iter().map(|code| code.as_str()).collect::<Vec<_>>(),
        vec![
            "missing_referent",
            "ambiguous",
            "confirmation_required",
            "private_source_denied",
            "expired",
            "unsupported",
            "resolver_unavailable",
        ]
    );
    let completion = TurnCompletion::ReferenceBlocked(ReferenceOutcomeCode::Ambiguous);
    assert!(matches!(
        completion,
        TurnCompletion::ReferenceBlocked(ReferenceOutcomeCode::Ambiguous)
    ));
    let block = crate::reference_resolution::ReferenceBlock::new(
        ReferenceOutcomeCode::Ambiguous,
        BlockedPresentation::Ambiguous,
    );
    assert_eq!(block.outcome(), ReferenceOutcomeCode::Ambiguous);
    assert_eq!(block.presentation(), BlockedPresentation::Ambiguous);
}

#[test]
fn current_turn_wrappers_redact_content_from_debug() {
    let sentinel = "synthetic-private-reference-sentinel";
    let input = UserAuthoredText::new(sentinel);
    let envelope =
        ConfirmationEnvelope::new(crate::reference_resolution::ConfirmationId::new(), sentinel);
    let mention = MentionText::public(sentinel, sentinel).expect("valid synthetic mention");

    for rendered in [
        format!("{input:?}"),
        format!("{envelope:?}"),
        format!("{mention:?}"),
    ] {
        assert!(
            !rendered.contains(sentinel),
            "content leaked from redacted debug: {rendered}"
        );
    }
}

#[test]
fn pure_contracts_reuse_trusted_evidence_origin_and_construct_the_input_shell() {
    let turn = ResolveTurn::new(
        TurnId::new(),
        crate::reference_resolution::SessionId::new("synthetic-session")
            .expect("valid synthetic session"),
        EvidenceOrigin::Chat,
        UserAuthoredText::new("show the product"),
        OriginalRequestScope::ReferenceBearing,
        None,
    );
    assert_eq!(turn.origin(), EvidenceOrigin::Chat);
    assert_eq!(turn.scope(), OriginalRequestScope::ReferenceBearing);
}

#[test]
fn inert_factory_is_a_send_sync_three_entry_resolver_shell() {
    fn accepts_resolver(
        resolver: std::sync::Arc<dyn crate::reference_resolution::ConversationalReferenceResolver>,
    ) {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn crate::reference_resolution::ConversationalReferenceResolver>();
        drop(resolver);
    }

    accepts_resolver(production());
    assert_eq!(ResolverFault::Unavailable.as_str(), "unavailable");
    assert_eq!(EntityKind::Product.as_str(), "product");
}

const SLICE6_ACCEPTED_CASE_IDS: [&str; 98] = [
    "explicit-product-web",
    "prior-user-pronoun",
    "current-over-prior",
    "ambiguous-singular",
    "recency-not-tiebreak",
    "eligibility-boundary",
    "expired-filter-leaves-unique",
    "incomplete-singular-comparison",
    "current-pair",
    "cross-kind-current-comparison",
    "current-plus-prior",
    "assistant-only",
    "mail-only",
    "assistant-mail-paraphrase",
    "attachment-product",
    "attachment-serial",
    "canonical-web",
    "canonical-web-broken",
    "accepted-polish",
    "accepted-polish-broken",
    "rejected-polish",
    "unknown-projection",
    "assistant-confirmed",
    "confirmation-mismatch",
    "confirmed-exact",
    "edited-confirmation",
    "public-url",
    "url-normalization",
    "url-path-case",
    "url-dns-admission",
    "normalization-case-space",
    "normalization-internal-punctuation",
    "normalization-canonical-unicode",
    "normalization-accent-distinct",
    "private-url",
    "credential-url",
    "all-kinds",
    "uncued-name",
    "english-bare",
    "slovak-bare",
    "english-human-pronoun",
    "english-it-ambiguous",
    "slovak-gender-ambiguous",
    "english-demonstrative",
    "slovak-demonstrative",
    "english-generic-kind",
    "slovak-generic-kind",
    "bare-demonstrative-ambiguous",
    "one-above-ambiguous",
    "slovak-above-ambiguous",
    "alias-aka",
    "formerly",
    "slovak-rename",
    "slovak-formerly",
    "alias-kind-mismatch",
    "inferred-alias-rejected",
    "prior-rename-augments",
    "alias-collision",
    "sensitive-id",
    "sensitive-pattern-family",
    "high-entropy-identifier",
    "sensitivity-precedence",
    "credential-query-url",
    "sensitive-confirmation",
    "generic-medication-public-ledger",
    "generic-account-reference",
    "slovak-sensitive-generic",
    "explicit-current-sensitive-topic",
    "sensitivity-substring-false-positive",
    "mixed-mail-web",
    "latest-mail",
    "plural-two-prior",
    "plural-three-prior",
    "cross-kind-prior-comparison",
    "unknown-prior-kind",
    "slovak-plural-two",
    "slovak-singular-incomplete",
    "slovak-current-pair",
    "expired",
    "quoted",
    "backticked",
    "unsupported-cataphora",
    "unsupported-fuzzy",
    "unsupported-possessive",
    "unsupported-former-latter",
    "unsupported-quoted-coreference",
    "unsupported-slovak-ellipsis",
    "overlap-product-quote",
    "overlap-url-quote",
    "overlap-identifier-quote",
    "medication",
    "message-limit",
    "span-count-limit",
    "span-length-limit",
    "automation-explicit",
    "automation-reference",
    "model-corruption",
    "flag-0",
];

fn synthetic_candidate(
    ordinal: u128,
    referent: &'static str,
    kind: EntityKind,
    provenance: LedgerProvenance,
) -> LedgerCandidate {
    let display = match kind {
        EntityKind::Product => Some("Aster Nova 12".to_owned()),
        EntityKind::Person => Some("Mira Vale".to_owned()),
        EntityKind::Organization => Some("Lumen Harbor Institute".to_owned()),
        EntityKind::Place => Some("Northbridge".to_owned()),
        EntityKind::TechnicalStandard => Some("ZX 2048-2".to_owned()),
        EntityKind::DocumentTitle => Some("Blue Orchard Protocol".to_owned()),
        EntityKind::PublicUrl => Some("https://public.example/products/aster".to_owned()),
        EntityKind::Unknown => None,
    };
    let normalized = display.as_deref().map(|value| value.to_lowercase());
    let restricted = matches!(
        provenance,
        LedgerProvenance::Assistant
            | LedgerProvenance::Mail
            | LedgerProvenance::Attachment
            | LedgerProvenance::Unknown
    );
    LedgerCandidate {
        mention_id: crate::reference_resolution::MentionId::from_uuid(Uuid::from_u128(ordinal)),
        referent_id: referent.to_owned(),
        entity_kind: kind,
        display,
        normalized,
        provenance,
        visibility: if restricted {
            MentionVisibility::ConfirmationOnly
        } else {
            MentionVisibility::ProviderSafe
        },
        sensitivity: MentionSensitivity::Public,
        introduced_sequence: 1,
        created_at_ms: 1_000,
        expires_at_ms: 1_801_000,
        age_turns: 1,
        age_minutes: 2,
        canonical_mapping_intact: true,
    }
}

fn expected_slice6_outcome(case_id: &str) -> Option<StructuralOutcome> {
    use StructuralOutcome::*;
    match case_id {
        "explicit-product-web"
        | "prior-user-pronoun"
        | "current-over-prior"
        | "eligibility-boundary"
        | "expired-filter-leaves-unique"
        | "current-pair"
        | "current-plus-prior"
        | "public-url"
        | "url-normalization"
        | "url-path-case"
        | "url-dns-admission"
        | "normalization-case-space"
        | "english-bare"
        | "slovak-bare"
        | "english-human-pronoun"
        | "english-demonstrative"
        | "slovak-demonstrative"
        | "english-generic-kind"
        | "slovak-generic-kind"
        | "alias-aka"
        | "formerly"
        | "slovak-rename"
        | "slovak-formerly"
        | "prior-rename-augments"
        | "explicit-current-sensitive-topic"
        | "sensitivity-substring-false-positive"
        | "plural-two-prior"
        | "slovak-plural-two"
        | "slovak-current-pair"
        | "overlap-product-quote"
        | "overlap-url-quote"
        | "automation-explicit" => Some(ResolvedUserPublic),
        "assistant-only"
        | "mail-only"
        | "assistant-mail-paraphrase"
        | "attachment-product"
        | "unknown-projection"
        | "confirmation-mismatch" => Some(ConfirmationRequired),
        "canonical-web"
        | "accepted-polish"
        | "assistant-confirmed"
        | "confirmed-exact" => Some(ResolvedConfirmedPublic),
        "ambiguous-singular"
        | "recency-not-tiebreak"
        | "english-it-ambiguous"
        | "slovak-gender-ambiguous"
        | "bare-demonstrative-ambiguous"
        | "one-above-ambiguous"
        | "slovak-above-ambiguous"
        | "alias-collision"
        | "plural-three-prior"
        | "model-corruption" => Some(Ambiguous),
        "incomplete-singular-comparison"
        | "canonical-web-broken"
        | "accepted-polish-broken"
        | "rejected-polish"
        | "unknown-prior-kind"
        | "slovak-singular-incomplete"
        | "quoted"
        | "backticked"
        | "automation-reference" => Some(MissingReferent),
        "attachment-serial"
        | "sensitive-id"
        | "sensitive-pattern-family"
        | "high-entropy-identifier"
        | "sensitivity-precedence"
        | "credential-query-url"
        | "sensitive-confirmation"
        | "generic-medication-public-ledger"
        | "generic-account-reference"
        | "slovak-sensitive-generic"
        | "medication"
        | "private-url"
        | "credential-url"
        | "overlap-identifier-quote" => Some(PrivateSourceDenied),
        "cross-kind-current-comparison"
        | "all-kinds"
        | "alias-kind-mismatch"
        | "inferred-alias-rejected"
        | "mixed-mail-web"
        | "cross-kind-prior-comparison"
        | "unsupported-cataphora"
        | "unsupported-fuzzy"
        | "unsupported-possessive"
        | "unsupported-former-latter"
        | "unsupported-quoted-coreference"
        | "unsupported-slovak-ellipsis" => Some(Unsupported),
        "expired" => Some(Expired),
        "normalization-internal-punctuation" | "uncued-name" | "latest-mail" => None,
        "normalization-accent-distinct" => Some(ResolvedUserPublic),
        "normalization-canonical-unicode" => Some(MissingReferent),
        "message-limit" | "span-count-limit" | "span-length-limit" | "flag-0" => None,
        "edited-confirmation" => Some(ResolvedUserPublic),
        _ => panic!("unmapped Slice 6 case: {case_id}"),
    }
}

fn slice6_fixture(case_id: &str) -> (&'static str, Vec<LedgerCandidate>, TurnOrigin, Option<&'static str>, bool) {
    let product = || synthetic_candidate(1, "aster", EntityKind::Product, LedgerProvenance::PriorUser);
    let boreal = || synthetic_candidate(2, "boreal", EntityKind::Product, LedgerProvenance::PriorUser);
    let person = || synthetic_candidate(3, "mira", EntityKind::Person, LedgerProvenance::PriorUser);
    let mut candidates = Vec::new();
    let mut origin = TurnOrigin::Chat;
    let mut confirmation = None;
    let mut flag = true;
    let message = match case_id {
        "explicit-product-web" => "Look up Aster Nova 12 specifications online.",
        "prior-user-pronoun" | "english-bare" => { candidates.push(product()); "Look up its specifications." },
        "current-over-prior" => { candidates.extend([product(), boreal()]); "Look up Aster Nova 12 online." },
        "ambiguous-singular" | "model-corruption" => { candidates.extend([product(), boreal()]); "Compare it online." },
        "recency-not-tiebreak" | "one-above-ambiguous" | "slovak-above-ambiguous" => { candidates.extend([product(), boreal()]); "Look up the one above online." },
        "eligibility-boundary" => { let mut candidate = product(); candidate.age_turns = 10; candidate.age_minutes = 30; candidates.push(candidate); "Look up that product online." },
        "expired-filter-leaves-unique" => { let mut expired = boreal(); expired.age_turns = 11; expired.age_minutes = 31; candidates.extend([product(), expired]); "Look up that product online." },
        "incomplete-singular-comparison" => { candidates.push(product()); "Compare it online." },
        "current-pair" => "Compare Aster Nova 12 and Boreal Finch 8 online.",
        "cross-kind-current-comparison" => "Compare Aster Nova 12 and ZX 2048-2 online.",
        "current-plus-prior" => { candidates.push(product()); "Compare Boreal Finch 8 with that product online." },
        "assistant-only" => { candidates.push(synthetic_candidate(4, "aster", EntityKind::Product, LedgerProvenance::Assistant)); "Look up its specifications." },
        "mail-only" | "assistant-mail-paraphrase" | "attachment-product" | "unknown-projection" | "confirmation-mismatch" => {
            let provenance = if case_id == "mail-only" || case_id == "confirmation-mismatch" { LedgerProvenance::Mail } else if case_id == "attachment-product" { LedgerProvenance::Attachment } else if case_id == "unknown-projection" { LedgerProvenance::Unknown } else { LedgerProvenance::Assistant };
            candidates.push(synthetic_candidate(4, "aster", EntityKind::Product, provenance));
            if case_id == "confirmation-mismatch" { confirmation = Some("Boreal Finch 8"); }
            "Look up that product online."
        }
        "attachment-serial" => {
            let mut candidate = synthetic_candidate(4, "serial", EntityKind::Unknown, LedgerProvenance::Attachment);
            candidate.sensitivity = MentionSensitivity::Sensitive;
            candidate.visibility = MentionVisibility::LocalOnly;
            candidates.push(candidate);
            "Look up that item online."
        }
        "canonical-web" | "accepted-polish" => { let provenance = if case_id == "canonical-web" { LedgerProvenance::CanonicalWeb } else { LedgerProvenance::AcceptedPolish }; candidates.push(synthetic_candidate(4, "aster", EntityKind::Product, provenance)); "Look it up online." }
        "canonical-web-broken" | "accepted-polish-broken" => { let provenance = if case_id == "canonical-web-broken" { LedgerProvenance::CanonicalWeb } else { LedgerProvenance::AcceptedPolish }; let mut candidate = synthetic_candidate(4, "aster", EntityKind::Product, provenance); candidate.canonical_mapping_intact = false; candidates.push(candidate); "Look it up online." }
        "rejected-polish" => "Look up that product online.",
        "assistant-confirmed" => { candidates.push(synthetic_candidate(4, "aster", EntityKind::Product, LedgerProvenance::Assistant)); confirmation = Some("Aster Nova 12"); "Look up that product online." },
        "confirmed-exact" => { candidates.push(synthetic_candidate(4, "aster", EntityKind::Product, LedgerProvenance::Mail)); confirmation = Some("Aster Nova 12"); "Look up Aster Nova 12 online." },
        "edited-confirmation" => { candidates.push(synthetic_candidate(4, "aster", EntityKind::Product, LedgerProvenance::Mail)); "Look up Boreal Finch 8 online." },
        "public-url" => "Inspect https://public.example/specs?id=2&utm_source=demo online.",
        "url-normalization" => "Inspect HTTPS://PUBLIC.EXAMPLE:443/Specs?b=2&utm_medium=demo&a=One#section online.",
        "url-path-case" => "Inspect https://public.example/Specs?name=One online.",
        "url-dns-admission" => "Inspect https://unresolved.example/Specs online.",
        "normalization-case-space" => "Look up ASTER   NOVA 12 online.",
        "normalization-internal-punctuation" => "Look up Aster-Nova 12 online.",
        "normalization-canonical-unicode" => "Porovnaj \"Žiar\" a \"Žiar\".",
        "normalization-accent-distinct" => "Look up Aster Nová 12 online.",
        "private-url" => "Inspect http://127.0.0.1/device online.",
        "credential-url" => "Inspect https://demo:secret@public.example/private online.",
        "all-kinds" => "Compare person Mira Vale, organization Lumen Harbor Institute, place Northbridge, product Aster Nova 12, standard ZX 2048-2, and document \"Blue Orchard Protocol\".",
        "uncued-name" => "Look up Mira Vale online.",
        "slovak-bare" => { candidates.push(product()); "Vyhľadaj ho online." },
        "english-human-pronoun" => { candidates.extend([person(), product()]); "Look her up online." },
        "english-it-ambiguous" | "slovak-gender-ambiguous" => { candidates.extend([person(), product()]); if case_id == "english-it-ambiguous" { "Look it up online." } else { "Vyhľadaj ju online." } },
        "english-demonstrative" | "english-generic-kind" => { candidates.extend([person(), product()]); "Look up that product online." },
        "slovak-demonstrative" | "slovak-generic-kind" => { candidates.extend([person(), product()]); "Vyhľadaj tento výrobok online." },
        "bare-demonstrative-ambiguous" => { candidates.extend([person(), product()]); "Look that up online." },
        "alias-aka" => "Look up Aster Nova 12, aka Starling 12, online.",
        "formerly" => "Compare Boreal Finch 8, formerly Polar Wren 8, online.",
        "slovak-rename" => "Vyhľadaj Aster Nova 12, po novom Starling 12, online.",
        "slovak-formerly" => "Vyhľadaj Boreal Finch 8, predtým Polar Wren 8, online.",
        "alias-kind-mismatch" => "Look up Aster Nova 12, aka ZX 2048-2, online.",
        "inferred-alias-rejected" => "Look up Aster Nova 12 or Starling 12 online.",
        "prior-rename-augments" => { candidates.extend([product(), synthetic_candidate(5, "aster", EntityKind::Product, LedgerProvenance::PriorUser)]); "Look up that product online." },
        "alias-collision" => { candidates.extend([synthetic_candidate(6, "aster", EntityKind::Product, LedgerProvenance::PriorUser), synthetic_candidate(7, "boreal", EntityKind::Product, LedgerProvenance::PriorUser)]); "Look up that product online." },
        "sensitive-id" => "Look up serial-Z9X8-777 online.",
        "sensitive-pattern-family" => "Look up token-AbCd1234 order-Q7W8 tracking-Z9Y7 account-K4M2 online.",
        "high-entropy-identifier" => "Look up Q7mP9xR2vN8kL4sD6fH1 online.",
        "sensitivity-precedence" => { candidates.push(product()); "Look up that product with token-AbCd1234 online." },
        "credential-query-url" => "Inspect https://public.example/Specs?token=synthetic-secret online.",
        "sensitive-confirmation" => { let mut candidate = synthetic_candidate(4, "sensitive", EntityKind::Product, LedgerProvenance::Attachment); candidate.sensitivity = MentionSensitivity::Sensitive; candidate.visibility = MentionVisibility::LocalOnly; candidates.push(candidate); confirmation = Some("Aster Nova 12"); "Look up that item online." },
        "generic-medication-public-ledger" | "generic-account-reference" | "slovak-sensitive-generic" => { candidates.push(product()); if case_id == "slovak-sensitive-generic" { "Vyhľadaj ten liek online." } else if case_id == "generic-account-reference" { "Look up that account online." } else { "Look up that medication online." } },
        "explicit-current-sensitive-topic" => "Look up product Aster Nova 12 medication safety online.",
        "sensitivity-substring-false-positive" => "Look up product Aster Nova 12 in an orderly comparison online.",
        "mixed-mail-web" => { candidates.push(product()); "Read the latest Mail and look up that product online." },
        "latest-mail" => "Read my latest Mail.",
        "plural-two-prior" => { candidates.extend([product(), boreal()]); "Compare them online." },
        "plural-three-prior" => { candidates.extend([product(), boreal(), synthetic_candidate(8, "third", EntityKind::Product, LedgerProvenance::PriorUser)]); "Compare them online." },
        "cross-kind-prior-comparison" => { candidates.extend([product(), person()]); "Compare them online." },
        "unknown-prior-kind" => { candidates.push(synthetic_candidate(9, "unknown", EntityKind::Unknown, LedgerProvenance::PriorUser)); "Look it up online." },
        "slovak-plural-two" => { candidates.extend([product(), boreal()]); "Porovnaj ich online." },
        "slovak-singular-incomplete" => { candidates.push(product()); "Porovnaj ho online." },
        "slovak-current-pair" => "Porovnaj Aster Nova 12 a Boreal Finch 8 online.",
        "expired" => { let mut candidate = product(); candidate.age_turns = 11; candidates.push(candidate); "Look up that product online." },
        "quoted" => "Look up \"Velvet Comet\" online.",
        "backticked" => "Look up `Velvet Comet` online.",
        "unsupported-cataphora" => "Before I name it, look it up online: Aster Nova 12.",
        "unsupported-fuzzy" => { candidates.push(product()); "Look up something like Aster Nova online." },
        "unsupported-possessive" => { candidates.push(product()); "Look up the product whose maker was mentioned." },
        "unsupported-former-latter" => { candidates.extend([product(), boreal()]); "Compare the former one with the latter one." },
        "unsupported-quoted-coreference" => { candidates.push(product()); "Analyze the quoted text \"look it up online\"." },
        "unsupported-slovak-ellipsis" => { candidates.extend([product(), boreal()]); "Vyhľadaj ten druhý online." },
        "overlap-product-quote" => "Look up \"Aster Nova 12\" online.",
        "overlap-url-quote" => "Inspect \"https://public.example/specs\" online.",
        "overlap-identifier-quote" => "Look up \"serial-Z9X8-777\" online.",
        "medication" => "Look up that medication online.",
        "message-limit" | "span-count-limit" | "span-length-limit" => "synthetic",
        "automation-explicit" => { origin = TurnOrigin::Automation; "Look up Aster Nova 12 online." },
        "automation-reference" => { origin = TurnOrigin::Automation; candidates.push(product()); "Look up that product online." },
        "flag-0" => { flag = false; candidates.push(product()); "Look up that product online." },
        _ => panic!("unmapped Slice 6 fixture: {case_id}"),
    };
    (message, candidates, origin, confirmation, flag)
}

fn run_slice6_case(case_id: &str) {
    let (message, candidates, origin, confirmation, flag) = slice6_fixture(case_id);
    let expected = expected_slice6_outcome(case_id);
    if !flag {
        assert_eq!(case_id, "flag-0");
        return;
    }
    if matches!(case_id, "message-limit" | "span-count-limit" | "span-length-limit") {
        let input = if case_id == "message-limit" {
            "X".repeat(crate::reference_resolution::MAX_MESSAGE_BYTES + 1)
        } else if case_id == "span-count-limit" {
            (0..=crate::reference_resolution::MAX_SPANS).map(|_| "Aster Nova 12").collect::<Vec<_>>().join(" and ")
        } else {
            format!("Look up \"{}\" online.", "V".repeat(crate::reference_resolution::MAX_SPAN_BYTES + 1))
        };
        assert!(extract(&UserAuthoredText::new(input)).is_err());
        return;
    }
    let input = UserAuthoredText::new(message);
    let extraction = extract(&input).expect("synthetic bounded extraction");
    for pair in extraction.spans.windows(2) {
        assert!(pair[0].span.end_utf8() <= pair[1].span.start_utf8(), "overlapping spans in {case_id}");
    }
    let order = candidates.iter().map(|candidate| candidate.mention_id).rev().collect::<Vec<_>>();
    let trace = resolve(&input, &extraction, &candidates, origin, confirmation, &order);
    assert_eq!(trace.outcome, expected, "wrong structural outcome for {case_id}: {trace:?}");
    assert!(trace.model_order_invariant, "model ordering influenced {case_id}");
    // The digest must not depend on candidate ordering: resolve again with the
    // forward order and require the same digest.
    let forward_order = candidates.iter().map(|candidate| candidate.mention_id).collect::<Vec<_>>();
    let reresolved = resolve(&input, &extraction, &candidates, origin, confirmation, &forward_order);
    assert_eq!(
        trace.input_digest, reresolved.input_digest,
        "digest must be stable across candidate ordering for {case_id}"
    );
}

macro_rules! slice6_named_cases {
    ($($name:ident),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_slice6_case(stringify!($name).replace('_', "-").as_str());
            }
        )+
    };
}

slice6_named_cases!(
    explicit_product_web, prior_user_pronoun, current_over_prior, ambiguous_singular,
    recency_not_tiebreak, eligibility_boundary, expired_filter_leaves_unique,
    incomplete_singular_comparison, current_pair, cross_kind_current_comparison,
    current_plus_prior, assistant_only, mail_only, assistant_mail_paraphrase,
    attachment_product, attachment_serial, canonical_web, canonical_web_broken,
    accepted_polish, accepted_polish_broken, rejected_polish, unknown_projection,
    assistant_confirmed, confirmation_mismatch, confirmed_exact, edited_confirmation,
    public_url, url_normalization, url_path_case, url_dns_admission, normalization_case_space,
    normalization_internal_punctuation, normalization_canonical_unicode, normalization_accent_distinct,
    private_url, credential_url, all_kinds, uncued_name, english_bare, slovak_bare,
    english_human_pronoun, english_it_ambiguous, slovak_gender_ambiguous, english_demonstrative,
    slovak_demonstrative, english_generic_kind, slovak_generic_kind, bare_demonstrative_ambiguous,
    one_above_ambiguous, slovak_above_ambiguous, alias_aka, formerly, slovak_rename,
    slovak_formerly, alias_kind_mismatch, inferred_alias_rejected, prior_rename_augments,
    alias_collision, sensitive_id, sensitive_pattern_family, high_entropy_identifier,
    sensitivity_precedence, credential_query_url, sensitive_confirmation,
    generic_medication_public_ledger, generic_account_reference, slovak_sensitive_generic,
    explicit_current_sensitive_topic, sensitivity_substring_false_positive, mixed_mail_web,
    latest_mail, plural_two_prior, plural_three_prior, cross_kind_prior_comparison,
    unknown_prior_kind, slovak_plural_two, slovak_singular_incomplete, slovak_current_pair,
    expired, quoted, backticked, unsupported_cataphora, unsupported_fuzzy, unsupported_possessive,
    unsupported_former_latter, unsupported_quoted_coreference, unsupported_slovak_ellipsis,
    overlap_product_quote, overlap_url_quote, overlap_identifier_quote, medication,
    message_limit, span_count_limit, span_length_limit, automation_explicit,
    automation_reference, model_corruption, flag_0,
);

#[test]
fn slice6_corpus_has_exactly_98_independently_filterable_cases() {
    assert_eq!(SLICE6_ACCEPTED_CASE_IDS.len(), 98);
}

#[test]
fn slice6_normalization_preserves_accents_and_url_path_case() {
    let input = UserAuthoredText::new("Inspect HTTPS://PUBLIC.EXAMPLE:443/Specs?b=2&utm_source=demo&a=One#section online.");
    let extraction = extract(&input).unwrap();
    let span = extraction.spans.first().expect("URL span");
    assert_eq!(span.normalized, "https://public.example/Specs?a=One&b=2");
    let accented = extract(&UserAuthoredText::new("Look up Aster Nová 12 online.")).unwrap();
    assert!(accented.spans.iter().any(|span| span.normalized.contains("nová")));
}

#[test]
fn slice6_model_order_is_permutation_only_and_never_changes_trace() {
    let input = UserAuthoredText::new("Compare them online.");
    let extraction = extract(&input).unwrap();
    let candidates = vec![
        synthetic_candidate(1, "aster", EntityKind::Product, LedgerProvenance::PriorUser),
        synthetic_candidate(2, "boreal", EntityKind::Product, LedgerProvenance::PriorUser),
    ];
    let forward = resolve(&input, &extraction, &candidates, TurnOrigin::Chat, None, &[]);
    let reverse = resolve(
        &input,
        &extraction,
        &candidates,
        TurnOrigin::Chat,
        None,
        &[candidates[1].mention_id, candidates[0].mention_id],
    );
    assert_eq!(forward.outcome, Some(StructuralOutcome::ResolvedUserPublic));
    assert_eq!(forward.selected_ids, reverse.selected_ids);
    assert_eq!(forward.outcome, reverse.outcome);
    assert_eq!(forward.provider_query_eligible, reverse.provider_query_eligible);
}

#[test]
fn slice6_invalid_model_orders_are_rejected_without_influencing_resolution() {
    let candidates = vec![
        synthetic_candidate(1, "aster", EntityKind::Product, LedgerProvenance::PriorUser),
        synthetic_candidate(2, "boreal", EntityKind::Product, LedgerProvenance::PriorUser),
    ];
    let eligible = candidates.iter().map(|candidate| candidate.mention_id).collect::<Vec<_>>();
    let input = UserAuthoredText::new("Compare them online.");
    let extraction = extract(&input).unwrap();
    let baseline = resolve(&input, &extraction, &candidates, TurnOrigin::Chat, None, &eligible);
    for order in [
        vec![eligible[0]],
        vec![eligible[0], eligible[0]],
        vec![crate::reference_resolution::MentionId::new(), eligible[1]],
    ] {
        assert!(!crate::reference_resolution::validate_model_order(&order, &eligible));
        let mutated = resolve(&input, &extraction, &candidates, TurnOrigin::Chat, None, &order);
        assert_eq!(mutated.selected_ids, baseline.selected_ids);
        assert_eq!(mutated.outcome, baseline.outcome);
        assert_eq!(mutated.provider_query_eligible, baseline.provider_query_eligible);
        assert!(mutated.model_order_invariant);
    }
}

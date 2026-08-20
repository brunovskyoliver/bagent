use crate::evidence::{self, BodyOrigin, CanonicalOutcomeStatus, EvidenceId, SourceAuthority, SourceIdentity};
use crate::reference_resolution::artifacts::{
    accept_polish, begin_turn, capture_current_user, finish_canonical_web,
    finish_legacy_assistant, finish_no_mention, finish_typed_attachment, finish_typed_mail,
    AttachmentSlot, CanonicalClaim, CanonicalMentionSlot, CanonicalSource, ClosedNoMentionReason,
    FinalOutputText, OutputArtifact, PolishSlot, SupportedTypedExtraction,
    TypedAttachmentSlot, TypedMailSlot,
};
use crate::reference_resolution::{
    extract, EntityKind, SessionId, TurnCompletion, TurnId, TurnOrigin, UserAuthoredText,
};
use url::Url;

fn session() -> SessionId {
    SessionId::new("producer-session").expect("synthetic session")
}

fn custody() -> crate::reference_resolution::crypto::CryptoCustody {
    crate::reference_resolution::crypto::CryptoCustody::with_provider(
        crate::reference_resolution::crypto::FakeKeyProvider::deterministic(),
    )
}

#[test]
fn current_user_capture_owns_public_visibility_and_exact_utf8_anchor() {
    let input = UserAuthoredText::new("Inspect Aster Nova 12 online.");
    let extraction = extract(&input).expect("synthetic extraction");
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let current = capture_current_user(
        execution.current_input_witness(),
        input,
        &extraction,
        &custody(),
        1_000,
    )
    .expect("current-user artifact");
    assert_eq!(current.graph().mentions.len(), 1);
    assert_eq!(current.graph().mentions[0].visibility, "provider_safe");
    assert_eq!(current.graph().anchors[0].start_utf8, Some(8));
    assert_eq!(current.graph().anchors[0].end_utf8, Some(21));
}

#[test]
fn current_user_capture_does_not_reparse_rewritten_text_or_upgrade_private_terms() {
    let original = UserAuthoredText::new("Inspect https://localhost/private online.");
    let extraction = extract(&original).expect("synthetic extraction");
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let error = capture_current_user(
        execution.current_input_witness(),
        UserAuthoredText::new("Inspect Aster Nova 12 online."),
        &extraction,
        &custody(),
        1_000,
    )
    .expect_err("rewritten bytes must not be reparsed");
    assert_eq!(error.as_str(), "binding_mismatch");

    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let current = capture_current_user(
        execution.current_input_witness(),
        original,
        &extraction,
        &custody(),
        1_000,
    )
    .expect("restricted current artifact");
    assert_eq!(current.graph().mentions[0].visibility, "local_only");
    assert_eq!(current.graph().mentions[0].sensitivity, "private");
    assert_eq!(current.graph().mentions[0].representation.kind().as_str(), "restricted");
}

#[test]
fn current_aliases_share_only_the_exact_typed_referent() {
    let input = UserAuthoredText::new("Look up Aster Nova 12, aka Starling 12, online.");
    let extraction = extract(&input).expect("synthetic extraction");
    assert!(extraction.alias);
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let current = capture_current_user(
        execution.current_input_witness(),
        input,
        &extraction,
        &custody(),
        1_000,
    )
    .expect("alias artifact");
    assert_eq!(current.graph().mentions.len(), 2);
    assert_eq!(
        current.graph().mentions[0].referent_id,
        current.graph().mentions[1].referent_id
    );
}

#[test]
fn canonical_web_requires_verified_complete_typed_allowlisted_mappings() {
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let evidence_id = EvidenceId::new("web-1").unwrap();
    let source = SourceIdentity::new("publisher-1").unwrap();
    let url = Url::parse("https://public.example/final").unwrap();
    let claim = CanonicalClaim::new("claim-1", vec![evidence_id.clone()]);
    let source_mapping = CanonicalSource::new(
        evidence_id.clone(),
        source,
        url,
        SourceAuthority::AuthoritativeReference,
        true,
    );
    let slot = CanonicalMentionSlot::new(
        "Aster Nova 12",
        EntityKind::Product,
        vec![8..21],
        "claim-1",
        vec![source_mapping],
    );
    let artifact = finish_canonical_web(
        execution.canonical_renderer_witness(),
        FinalOutputText::new("Inspect Aster Nova 12 [Source](https://public.example/final)."),
        vec![slot],
        vec![claim],
        CanonicalOutcomeStatus::Verified,
        true,
        &custody(),
        1_000,
    )
    .expect("verified canonical artifact");
    assert_eq!(artifact.graph().mentions.len(), 1);
    assert_eq!(artifact.graph().web_mappings.len(), 1);
    assert_eq!(artifact.graph().mentions[0].provenance, "web_evidence");
}

#[test]
fn canonical_web_rejects_inconsistent_claims_and_first_party_owner_mismatch() {
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let mapping = CanonicalSource::new(
        EvidenceId::new("web-1").unwrap(),
        SourceIdentity::new("publisher-1").unwrap(),
        Url::parse("https://public.example/final").unwrap(),
        SourceAuthority::FirstParty,
        false,
    );
    let slot = CanonicalMentionSlot::new(
        "Aster Nova 12",
        EntityKind::Product,
        vec![0..13],
        "missing-claim",
        vec![mapping],
    );
    let error = finish_canonical_web(
        execution.canonical_renderer_witness(),
        FinalOutputText::new("Aster Nova 12"),
        vec![slot],
        vec![CanonicalClaim::new("claim-1", vec![EvidenceId::new("web-1").unwrap()])],
        CanonicalOutcomeStatus::Verified,
        true,
        &custody(),
        1_000,
    )
    .expect_err("owner mismatch and claim mismatch must reject");
    assert_eq!(error.as_str(), "incomplete_lineage");
}

#[test]
fn canonical_web_requires_the_status_and_source_threshold_for_reuse() {
    for status in [
        CanonicalOutcomeStatus::Conflict,
        CanonicalOutcomeStatus::Partial,
        CanonicalOutcomeStatus::VerificationShortfall,
    ] {
        let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
        let error = finish_canonical_web(
            execution.canonical_renderer_witness(),
            FinalOutputText::new("Aster Nova 12 https://one.example/final"),
            vec![CanonicalMentionSlot::new(
                "Aster Nova 12",
                EntityKind::Product,
                vec![0..13],
                "claim-1",
                vec![CanonicalSource::new(
                    EvidenceId::new("web-1").unwrap(),
                    SourceIdentity::new("one").unwrap(),
                    Url::parse("https://one.example/final").unwrap(),
                    SourceAuthority::Other,
                    false,
                )],
            )],
            vec![CanonicalClaim::new(
                "claim-1",
                vec![EvidenceId::new("web-1").unwrap()],
            )],
            status,
            true,
            &custody(),
            1_000,
        )
        .expect_err("non-verified canonical status must not create a mention");
        assert_eq!(error.as_str(), "policy_rejected");
    }

    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let one_source = finish_canonical_web(
        execution.canonical_renderer_witness(),
        FinalOutputText::new("Aster Nova 12 https://one.example/final"),
        vec![CanonicalMentionSlot::new(
            "Aster Nova 12",
            EntityKind::Product,
            vec![0..13],
            "claim-1",
            vec![CanonicalSource::new(
                EvidenceId::new("web-1").unwrap(),
                SourceIdentity::new("one").unwrap(),
                Url::parse("https://one.example/final").unwrap(),
                SourceAuthority::Other,
                false,
            )],
        )],
        vec![CanonicalClaim::new(
            "claim-1",
            vec![EvidenceId::new("web-1").unwrap()],
        )],
        CanonicalOutcomeStatus::Verified,
        true,
        &custody(),
        1_000,
    )
    .expect_err("Other requires two independent source identities");
    assert_eq!(one_source.as_str(), "policy_rejected");

    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let two_sources = finish_canonical_web(
        execution.canonical_renderer_witness(),
        FinalOutputText::new(
            "Aster Nova 12 https://one.example/final https://two.example/final",
        ),
        vec![CanonicalMentionSlot::new(
            "Aster Nova 12",
            EntityKind::Product,
            vec![0..13],
            "claim-1",
            vec![
                CanonicalSource::new(
                    EvidenceId::new("web-1").unwrap(),
                    SourceIdentity::new("one").unwrap(),
                    Url::parse("https://one.example/final").unwrap(),
                    SourceAuthority::Other,
                    false,
                ),
                CanonicalSource::new(
                    EvidenceId::new("web-2").unwrap(),
                    SourceIdentity::new("two").unwrap(),
                    Url::parse("https://two.example/final").unwrap(),
                    SourceAuthority::Other,
                    false,
                ),
            ],
        )],
        vec![CanonicalClaim::new(
            "claim-1",
            vec![EvidenceId::new("web-1").unwrap(), EvidenceId::new("web-2").unwrap()],
        )],
        CanonicalOutcomeStatus::Verified,
        true,
        &custody(),
        1_000,
    )
    .expect("two independent Other sources qualify");
    assert_eq!(two_sources.graph().mentions.len(), 1);
}

#[test]
fn accepted_polish_preserves_each_canonical_slot_and_rejected_polish_is_inert() {
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let evidence_id = EvidenceId::new("web-1").unwrap();
    let canonical = finish_canonical_web(
        execution.canonical_renderer_witness(),
        FinalOutputText::new("Aster Nova 12 [Source](https://public.example/final)."),
        vec![CanonicalMentionSlot::new(
            "Aster Nova 12",
            EntityKind::Product,
            vec![0..13],
            "claim-1",
            vec![CanonicalSource::new(
                evidence_id.clone(),
                SourceIdentity::new("publisher-1").unwrap(),
                Url::parse("https://public.example/final").unwrap(),
                SourceAuthority::AuthoritativeReference,
                true,
            )],
        )],
        vec![CanonicalClaim::new("claim-1", vec![evidence_id])],
        CanonicalOutcomeStatus::Verified,
        true,
        &custody(),
        1_000,
    )
    .unwrap();
    let polished = accept_polish(
        execution.accepted_polish_witness(),
        &canonical,
        FinalOutputText::new("Aster Nova 12 — verified."),
        vec![PolishSlot::from_canonical(&canonical, 0, 0..13)],
        &custody(),
        1_000,
    )
    .expect("accepted polish");
    assert_eq!(polished.graph().derivations.len(), 1);

    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let canonical = finish_canonical_web(
        execution.canonical_renderer_witness(),
        FinalOutputText::new("Aster Nova 12 [Source](https://public.example/final)."),
        vec![CanonicalMentionSlot::new(
            "Aster Nova 12",
            EntityKind::Product,
            vec![0..13],
            "claim-1",
            vec![CanonicalSource::new(
                EvidenceId::new("web-1").unwrap(),
                SourceIdentity::new("publisher-1").unwrap(),
                Url::parse("https://public.example/final").unwrap(),
                SourceAuthority::AuthoritativeReference,
                true,
            )],
        )],
        vec![CanonicalClaim::new("claim-1", vec![EvidenceId::new("web-1").unwrap()])],
        CanonicalOutcomeStatus::Verified,
        true,
        &custody(),
        1_000,
    )
    .unwrap();
    let error = accept_polish(
        execution.accepted_polish_witness(),
        &canonical,
        FinalOutputText::new("Aster Nova 12 — verified."),
        vec![],
        &custody(),
        1_000,
    )
    .expect_err("omitted slot must reject polish");
    assert_eq!(error.as_str(), "policy_rejected");
}

#[test]
fn mail_and_attachment_producers_preserve_taint_and_drop_private_metadata() {
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let mail = finish_typed_mail(
        execution.mail_renderer_witness(),
        FinalOutputText::new("Aster Nova 12"),
        vec![TypedMailSlot::new(
            "Aster Nova 12",
            EntityKind::Product,
            vec![0..13],
            evidence::MailBodyEvidence {
                evidence_id: EvidenceId::new("body-1").unwrap(),
                header_id: EvidenceId::new("header-1").unwrap(),
                body: "Aster Nova 12 is available.".into(),
                body_state: evidence::BodyState::Readable,
                body_origin: BodyOrigin::LocalEmlx,
            },
        )],
        &custody(),
        1_000,
    )
    .expect("safe mail product");
    assert_eq!(mail.graph().mentions[0].provenance, "mail_evidence");
    assert_eq!(mail.graph().mentions[0].visibility, "confirmation_only");
    assert_eq!(mail.graph().mentions[0].mail_body_origin.as_deref(), Some("local_emlx"));
    let debug = format!("{mail:?}");
    for forbidden in ["header-1", "sender", "subject", "filename", "path"] {
        assert!(!debug.contains(forbidden));
    }

    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let attachment = finish_typed_attachment(
        execution.attachment_witness(),
        FinalOutputText::new("Aster Nova 12"),
        vec![TypedAttachmentSlot::new("Aster Nova 12", EntityKind::Product, vec![0..13])],
        SupportedTypedExtraction::new("opaque-attachment-1", "Aster Nova 12"),
        &custody(),
        1_000,
    )
    .expect("safe attachment product");
    assert_eq!(attachment.graph().mentions[0].provenance, "attachment_evidence");
    assert_eq!(attachment.graph().mentions[0].visibility, "confirmation_only");
    let debug = format!("{attachment:?}");
    assert!(!debug.contains("opaque-attachment-1"));

    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let mail = finish_typed_mail(
        execution.mail_renderer_witness(),
        FinalOutputText::new("Order 12345"),
        vec![TypedMailSlot::new(
            "Order 12345",
            EntityKind::Product,
            vec![0..11],
            evidence::MailBodyEvidence {
                evidence_id: EvidenceId::new("body-2").unwrap(),
                header_id: EvidenceId::new("header-2").unwrap(),
                body: "Order 12345".into(),
                body_state: evidence::BodyState::Readable,
                body_origin: BodyOrigin::MailAutomation,
            },
        )],
        &custody(),
        1_000,
    )
    .expect("mail producer closes private identifier");
    assert!(mail.graph().mentions.is_empty());

    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let attachment = finish_typed_attachment(
        execution.attachment_witness(),
        FinalOutputText::new("Serial 12345"),
        vec![TypedAttachmentSlot::new(
            "Serial 12345",
            EntityKind::Product,
            vec![0..12],
        )],
        SupportedTypedExtraction::new("opaque-attachment-3", "Serial 12345"),
        &custody(),
        1_000,
    )
    .expect("attachment producer closes private identifier");
    assert!(attachment.graph().mentions.is_empty());
}

#[test]
fn legacy_and_deterministic_no_mention_outputs_are_explicit_and_one_use() {
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let legacy = finish_legacy_assistant(
        execution.legacy_terminal_witness(),
        FinalOutputText::new("Aster Nova 12"),
        &custody(),
        1_000,
    );
    assert!(legacy.graph().mentions.is_empty());
    let no_mention = finish_no_mention(
        execution.deterministic_terminal_witness(),
        ClosedNoMentionReason::LegacyAssistantOutput,
    );
    assert!(no_mention.graph().mentions.is_empty());
    let _completed = execution
        .seal(
            crate::reference_resolution::artifacts::CurrentUserMentionArtifact::empty_for_test(),
            OutputArtifact::LegacyAssistant(legacy),
            TurnCompletion::Completed,
        )
        .expect("single terminal seal");
}

#[test]
fn cross_turn_artifacts_and_duplicate_terminal_finalization_fail_closed() {
    let mut first = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let second = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let output = finish_no_mention(
        first.deterministic_terminal_witness(),
        ClosedNoMentionReason::NoQualifyingSlot,
    );
    let error = second
        .seal(
            crate::reference_resolution::artifacts::CurrentUserMentionArtifact::empty_for_test(),
            OutputArtifact::NoMention(output),
            TurnCompletion::Completed,
        )
        .expect_err("cross-turn artifact must fail");
    assert_eq!(error.as_str(), "binding_mismatch");
}

#[test]
fn unavailable_body_origin_and_unsupported_attachment_create_no_projection() {
    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let mail = finish_typed_mail(
        execution.mail_renderer_witness(),
        FinalOutputText::new("Aster Nova 12"),
        vec![TypedMailSlot::new(
            "Aster Nova 12",
            EntityKind::Product,
            vec![0..13],
            evidence::MailBodyEvidence {
                evidence_id: EvidenceId::new("body-1").unwrap(),
                header_id: EvidenceId::new("header-1").unwrap(),
                body: "Aster Nova 12".into(),
                body_state: evidence::BodyState::UnavailableLocally,
                body_origin: BodyOrigin::Unavailable,
            },
        )],
        &custody(),
        1_000,
    )
    .expect("mail producer closes without projection");
    assert!(mail.graph().mentions.is_empty());

    let mut execution = begin_turn(TurnId::new(), session(), TurnOrigin::Chat);
    let attachment = finish_typed_attachment(
        execution.attachment_witness(),
        FinalOutputText::new("Aster Nova 12"),
        vec![AttachmentSlot::new("Aster Nova 12", EntityKind::Product, vec![0..13])],
        SupportedTypedExtraction::unsupported("opaque-attachment-2"),
        &custody(),
        1_000,
    )
    .expect("attachment producer closes without projection");
    assert!(attachment.graph().mentions.is_empty());
}

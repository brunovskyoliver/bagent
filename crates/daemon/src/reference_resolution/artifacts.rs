use super::crypto::{AadBinding, CryptoCustody};
use super::extraction::Extraction;
use super::types::{
    EntityKind, MentionSensitivity, SessionId, TurnCompletion, TurnId, TurnOrigin, UserAuthoredText,
};
use crate::evidence::{
    BodyOrigin, BodyState, CanonicalOutcomeStatus, EvidenceId, MailBodyEvidence, SourceAuthority,
    SourceIdentity,
};
use sha2::Digest;
use std::{fmt, ops::Range};
use url::Url;
use uuid::Uuid;

const DIGEST_SIZE: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MentionRepresentationKind {
    PublicVisible,
    Restricted,
    Opaque,
}

impl MentionRepresentationKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::PublicVisible => "public_visible",
            Self::Restricted => "restricted",
            Self::Opaque => "opaque",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum MentionRepresentation {
    PublicVisible { display: String, normalized: String },
    Restricted { span_hmac: [u8; DIGEST_SIZE] },
    Opaque { fingerprint: [u8; DIGEST_SIZE] },
}

impl MentionRepresentation {
    pub(super) const fn kind(&self) -> MentionRepresentationKind {
        match self {
            Self::PublicVisible { .. } => MentionRepresentationKind::PublicVisible,
            Self::Restricted { .. } => MentionRepresentationKind::Restricted,
            Self::Opaque { .. } => MentionRepresentationKind::Opaque,
        }
    }

    pub(super) fn normalized_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::PublicVisible { normalized, .. } => Some(normalized.as_bytes()),
            Self::Restricted { span_hmac } => Some(span_hmac),
            Self::Opaque { fingerprint } => Some(fingerprint),
        }
    }
}

impl fmt::Debug for MentionRepresentation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MentionRepresentation")
            .field("kind", &self.kind().as_str())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DerivationKind {
    CanonicalRenderOf,
    AcceptedPolishOf,
    ExactStructuredRepeatOf,
    SafeProjectionOf,
}

impl DerivationKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalRenderOf => "canonical_render_of",
            Self::AcceptedPolishOf => "accepted_polish_of",
            Self::ExactStructuredRepeatOf => "exact_structured_repeat_of",
            Self::SafeProjectionOf => "safe_projection_of",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnchorKind {
    Visible,
    Opaque,
}

impl AnchorKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Opaque => "opaque",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisplayClass {
    UserInput,
    Canonical,
    AcceptedPolish,
    RestrictedOutput,
    Opaque,
}

impl DisplayClass {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::UserInput => "user_input",
            Self::Canonical => "canonical",
            Self::AcceptedPolish => "accepted_polish",
            Self::RestrictedOutput => "restricted_output",
            Self::Opaque => "opaque",
        }
    }
}

#[derive(Clone)]
pub(super) struct MentionArtifact {
    pub(super) mention_id: String,
    pub(super) referent_id: String,
    pub(super) turn_id: String,
    pub(super) session_id: String,
    pub(super) canonical_parent_mention_id: Option<String>,
    pub(super) entity_kind: String,
    pub(super) provenance: String,
    pub(super) assistant_lineage: Option<String>,
    pub(super) producer: String,
    pub(super) visibility: String,
    pub(super) sensitivity: String,
    pub(super) direct_user: bool,
    pub(super) untrusted_evidence: bool,
    pub(super) origin_ref_hmac: Option<[u8; DIGEST_SIZE]>,
    pub(super) mail_body_origin: Option<String>,
    pub(super) representation: MentionRepresentation,
    pub(super) created_at_ms: i64,
    pub(super) expires_at_ms: i64,
    pub(super) hmac_key_version: u32,
    pub(super) encryption_key_version: Option<u32>,
}

impl fmt::Debug for MentionArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MentionArtifact")
            .field("mention_id", &"<redacted>")
            .field("representation", &self.representation)
            .field("provenance", &self.provenance)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub(super) struct DerivationArtifact {
    pub(super) derivation_id: String,
    pub(super) derived_mention_id: String,
    pub(super) parent_mention_id: String,
    pub(super) kind: DerivationKind,
    pub(super) parent_ordinal: i64,
    pub(super) created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(super) struct AnchorArtifact {
    pub(super) anchor_id: String,
    pub(super) mention_id: String,
    pub(super) turn_id: String,
    pub(super) kind: AnchorKind,
    pub(super) display_class: DisplayClass,
    pub(super) ordinal: i64,
    pub(super) start_utf8: Option<i64>,
    pub(super) end_utf8: Option<i64>,
    pub(super) visible_span_hmac: Option<[u8; DIGEST_SIZE]>,
    pub(super) opaque_anchor_hmac: Option<[u8; DIGEST_SIZE]>,
    pub(super) hmac_key_version: u32,
    pub(super) created_at_ms: i64,
}

#[derive(Clone)]
pub(super) struct WebMappingArtifact {
    pub(super) mapping_id: String,
    pub(super) mention_id: String,
    pub(super) canonical_anchor_id: String,
    pub(super) source_ordinal: i64,
    pub(super) evidence_id_hmac: [u8; DIGEST_SIZE],
    pub(super) source_identity: String,
    pub(super) source_identity_hmac: [u8; DIGEST_SIZE],
    pub(super) public_url: String,
    pub(super) public_url_hmac: [u8; DIGEST_SIZE],
    pub(super) authority: String,
    pub(super) network_policy_version: u32,
    pub(super) validated_at_ms: i64,
    pub(super) encryption_key_version: u32,
    pub(super) hmac_key_version: u32,
}

impl fmt::Debug for WebMappingArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebMappingArtifact")
            .field("mapping_id", &"<redacted>")
            .field("source_identity", &"<redacted>")
            .field("public_url", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ArtifactGraph {
    pub(super) mentions: Vec<MentionArtifact>,
    pub(super) derivations: Vec<DerivationArtifact>,
    pub(super) anchors: Vec<AnchorArtifact>,
    pub(super) web_mappings: Vec<WebMappingArtifact>,
}

impl ArtifactGraph {
    pub(super) fn canonical_parts(&self) -> Vec<Vec<u8>> {
        let mut parts = Vec::new();
        for mention in &self.mentions {
            let mut part = Vec::new();
            push_string(&mut part, &mention.mention_id);
            push_string(&mut part, &mention.referent_id);
            push_string(&mut part, &mention.turn_id);
            push_string(&mut part, &mention.session_id);
            push_string(
                &mut part,
                mention.canonical_parent_mention_id.as_deref().unwrap_or(""),
            );
            push_string(&mut part, &mention.entity_kind);
            push_string(&mut part, &mention.provenance);
            push_string(
                &mut part,
                mention.assistant_lineage.as_deref().unwrap_or(""),
            );
            push_string(&mut part, &mention.producer);
            push_string(&mut part, &mention.visibility);
            push_string(&mut part, &mention.sensitivity);
            part.push(u8::from(mention.direct_user));
            part.push(u8::from(mention.untrusted_evidence));
            part.extend_from_slice(
                mention
                    .origin_ref_hmac
                    .as_ref()
                    .unwrap_or(&[0; DIGEST_SIZE]),
            );
            push_string(&mut part, mention.mail_body_origin.as_deref().unwrap_or(""));
            part.push(mention.representation.kind().as_str().as_bytes()[0]);
            match &mention.representation {
                MentionRepresentation::PublicVisible {
                    display,
                    normalized,
                } => {
                    push_string(&mut part, display);
                    push_string(&mut part, normalized);
                }
                _ => {
                    if let Some(bytes) = mention.representation.normalized_bytes() {
                        part.extend_from_slice(bytes);
                    }
                }
            }
            part.extend_from_slice(&mention.created_at_ms.to_be_bytes());
            part.extend_from_slice(&mention.expires_at_ms.to_be_bytes());
            parts.push(part);
        }
        for derivation in &self.derivations {
            let mut part = Vec::new();
            push_string(&mut part, &derivation.derivation_id);
            push_string(&mut part, &derivation.derived_mention_id);
            push_string(&mut part, &derivation.parent_mention_id);
            push_string(&mut part, derivation.kind.as_str());
            part.extend_from_slice(&derivation.parent_ordinal.to_be_bytes());
            parts.push(part);
        }
        for anchor in &self.anchors {
            let mut part = Vec::new();
            push_string(&mut part, &anchor.anchor_id);
            push_string(&mut part, &anchor.mention_id);
            push_string(&mut part, &anchor.turn_id);
            push_string(&mut part, anchor.kind.as_str());
            push_string(&mut part, anchor.display_class.as_str());
            part.extend_from_slice(&anchor.ordinal.to_be_bytes());
            part.extend_from_slice(&anchor.start_utf8.unwrap_or(-1).to_be_bytes());
            part.extend_from_slice(&anchor.end_utf8.unwrap_or(-1).to_be_bytes());
            part.extend_from_slice(
                anchor
                    .visible_span_hmac
                    .as_ref()
                    .unwrap_or(&[0; DIGEST_SIZE]),
            );
            part.extend_from_slice(
                anchor
                    .opaque_anchor_hmac
                    .as_ref()
                    .unwrap_or(&[0; DIGEST_SIZE]),
            );
            parts.push(part);
        }
        for mapping in &self.web_mappings {
            let mut part = Vec::new();
            push_string(&mut part, &mapping.mapping_id);
            push_string(&mut part, &mapping.mention_id);
            push_string(&mut part, &mapping.canonical_anchor_id);
            part.extend_from_slice(&mapping.source_ordinal.to_be_bytes());
            part.extend_from_slice(&mapping.evidence_id_hmac);
            part.extend_from_slice(&mapping.source_identity_hmac);
            part.extend_from_slice(&mapping.public_url_hmac);
            push_string(&mut part, &mapping.authority);
            part.extend_from_slice(&mapping.network_policy_version.to_be_bytes());
            part.extend_from_slice(&mapping.validated_at_ms.to_be_bytes());
            parts.push(part);
        }
        parts
    }
}

fn push_string(buffer: &mut Vec<u8>, value: &str) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value.as_bytes());
}

pub(super) fn validate_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .map(|uuid| uuid.to_string() == value)
        .unwrap_or(false)
}

/// A normalized failure class for producer-owned capture.  It intentionally
/// contains no source text or caller-provided policy values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureError {
    BindingMismatch,
    MalformedArtifact,
    IncompleteLineage,
    PolicyRejected,
    UnsupportedProducerState,
    NonTerminal,
}

impl CaptureError {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::BindingMismatch => "binding_mismatch",
            Self::MalformedArtifact => "malformed_artifact",
            Self::IncompleteLineage => "incomplete_lineage",
            Self::PolicyRejected => "policy_rejected",
            Self::UnsupportedProducerState => "unsupported_producer_state",
            Self::NonTerminal => "non_terminal",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct TurnBinding {
    turn_id: TurnId,
    session_id: SessionId,
    origin: TurnOrigin,
}

impl fmt::Debug for TurnBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TurnBinding")
            .field("turn_id", &self.turn_id)
            .field("origin", &self.origin)
            .finish()
    }
}

macro_rules! witness {
    ($name:ident) => {
        pub(super) struct $name {
            binding: TurnBinding,
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(stringify!($name))
            }
        }
    };
}

witness!(CurrentInputWitness);
witness!(CanonicalRendererWitness);
witness!(AcceptedPolishWitness);
witness!(CanonicalMailRendererWitness);
witness!(AttachmentExtractionWitness);
witness!(AcceptedLegacyTerminalWitness);
witness!(DeterministicTerminalWitness);

/// The daemon owns this execution token.  Each producer witness is minted at
/// most once and is consumed by its matching finalizer.
pub(super) struct TurnExecution {
    binding: TurnBinding,
    current_input: bool,
    canonical_renderer: bool,
    accepted_polish: bool,
    mail_renderer: bool,
    attachment: bool,
    legacy_terminal: bool,
    deterministic_terminal: bool,
}

pub(super) fn begin_turn(
    turn_id: TurnId,
    session_id: SessionId,
    origin: TurnOrigin,
) -> TurnExecution {
    TurnExecution {
        binding: TurnBinding {
            turn_id,
            session_id,
            origin,
        },
        current_input: true,
        canonical_renderer: true,
        accepted_polish: true,
        mail_renderer: true,
        attachment: true,
        legacy_terminal: true,
        deterministic_terminal: true,
    }
}

macro_rules! witness_method {
    ($method:ident, $field:ident, $witness:ident) => {
        pub(super) fn $method(&mut self) -> $witness {
            assert!(self.$field, "producer witness already consumed");
            self.$field = false;
            $witness {
                binding: self.binding.clone(),
            }
        }
    };
}

impl TurnExecution {
    witness_method!(current_input_witness, current_input, CurrentInputWitness);
    witness_method!(
        canonical_renderer_witness,
        canonical_renderer,
        CanonicalRendererWitness
    );
    witness_method!(
        accepted_polish_witness,
        accepted_polish,
        AcceptedPolishWitness
    );
    witness_method!(
        mail_renderer_witness,
        mail_renderer,
        CanonicalMailRendererWitness
    );
    witness_method!(attachment_witness, attachment, AttachmentExtractionWitness);
    witness_method!(
        legacy_terminal_witness,
        legacy_terminal,
        AcceptedLegacyTerminalWitness
    );
    witness_method!(
        deterministic_terminal_witness,
        deterministic_terminal,
        DeterministicTerminalWitness
    );

    pub(super) fn seal(
        self,
        current_user: CurrentUserMentionArtifact,
        output: OutputArtifact,
        terminal: TurnCompletion,
    ) -> Result<CompletedTurnArtifacts, CaptureError> {
        if !matches!(terminal, TurnCompletion::Completed) {
            return Err(CaptureError::NonTerminal);
        }
        if current_user.binding.is_some() && current_user.binding.as_ref() != Some(&self.binding) {
            return Err(CaptureError::BindingMismatch);
        }
        if !output.matches(&self.binding) {
            return Err(CaptureError::BindingMismatch);
        }
        let mut graph = current_user.graph.clone();
        graph.extend(output.graph());
        if !graph_is_acyclic(&graph) {
            return Err(CaptureError::MalformedArtifact);
        }
        Ok(CompletedTurnArtifacts {
            binding: self.binding,
            input: current_user.input.clone(),
            current_user,
            output,
            terminal,
            artifact_digest: graph.digest(),
            graph,
        })
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct FinalOutputText(String);

impl FinalOutputText {
    pub(super) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Debug for FinalOutputText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FinalOutputText(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CanonicalClaim {
    id: String,
    evidence_ids: Vec<EvidenceId>,
}

impl CanonicalClaim {
    pub(super) fn new(id: impl Into<String>, evidence_ids: Vec<EvidenceId>) -> Self {
        Self {
            id: id.into(),
            evidence_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CanonicalSource {
    evidence_id: EvidenceId,
    source_identity: SourceIdentity,
    final_url: Url,
    authority: SourceAuthority,
    page_owner_entity_bound: bool,
}

impl CanonicalSource {
    pub(super) fn new(
        evidence_id: EvidenceId,
        source_identity: SourceIdentity,
        final_url: Url,
        authority: SourceAuthority,
        page_owner_entity_bound: bool,
    ) -> Self {
        Self {
            evidence_id,
            source_identity,
            final_url,
            authority,
            page_owner_entity_bound,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CanonicalMentionSlot {
    display: String,
    normalized: String,
    entity_kind: EntityKind,
    spans: Vec<Range<usize>>,
    claim_id: String,
    sources: Vec<CanonicalSource>,
}

impl CanonicalMentionSlot {
    pub(super) fn new(
        display: impl Into<String>,
        entity_kind: EntityKind,
        spans: Vec<Range<usize>>,
        claim_id: impl Into<String>,
        sources: Vec<CanonicalSource>,
    ) -> Self {
        let display = display.into();
        Self {
            normalized: normalize_term(&display),
            display,
            entity_kind,
            spans,
            claim_id: claim_id.into(),
            sources,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PolishSlot {
    index: usize,
    display: String,
    normalized: String,
    entity_kind: EntityKind,
    spans: Vec<Range<usize>>,
    claim_id: String,
    source_signature: Vec<([u8; 32], String)>,
}

impl PolishSlot {
    pub(super) fn from_canonical(
        canonical: &CanonicalWebArtifact,
        index: usize,
        span: Range<usize>,
    ) -> Self {
        let slot = &canonical.slots[index];
        Self {
            index,
            display: slot.display.clone(),
            normalized: slot.normalized.clone(),
            entity_kind: slot.entity_kind,
            spans: vec![span],
            claim_id: slot.claim_id.clone(),
            source_signature: slot.source_signature.clone(),
        }
    }
}

pub(super) type MailSlot = TypedMailSlot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TypedMailSlot {
    display: String,
    entity_kind: EntityKind,
    spans: Vec<Range<usize>>,
    evidence: MailBodyEvidence,
}

impl TypedMailSlot {
    pub(super) fn new(
        display: impl Into<String>,
        entity_kind: EntityKind,
        spans: Vec<Range<usize>>,
        evidence: MailBodyEvidence,
    ) -> Self {
        Self {
            display: display.into(),
            entity_kind,
            spans,
            evidence,
        }
    }
}

pub(super) type AttachmentSlot = TypedAttachmentSlot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TypedAttachmentSlot {
    display: String,
    entity_kind: EntityKind,
    spans: Vec<Range<usize>>,
}

impl TypedAttachmentSlot {
    pub(super) fn new(
        display: impl Into<String>,
        entity_kind: EntityKind,
        spans: Vec<Range<usize>>,
    ) -> Self {
        Self {
            display: display.into(),
            entity_kind,
            spans,
        }
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct SupportedTypedExtraction {
    attachment_ref: String,
    text: String,
    supported: bool,
}

impl SupportedTypedExtraction {
    pub(super) fn new(attachment_ref: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            attachment_ref: attachment_ref.into(),
            text: text.into(),
            supported: true,
        }
    }

    pub(super) fn unsupported(attachment_ref: impl Into<String>) -> Self {
        Self {
            attachment_ref: attachment_ref.into(),
            text: String::new(),
            supported: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClosedNoMentionReason {
    LegacyAssistantOutput,
    NoQualifyingSlot,
    VerificationShortfall,
    UnsupportedProducerState,
    Blocked,
}

#[derive(Clone)]
struct SlotRecord {
    mention_id: String,
    display: String,
    normalized: String,
    entity_kind: EntityKind,
    claim_id: String,
    source_signature: Vec<([u8; 32], String)>,
}

pub(super) struct CurrentUserMentionArtifact {
    binding: Option<TurnBinding>,
    input: UserAuthoredText,
    graph: ArtifactGraph,
}

pub(super) struct CanonicalWebArtifact {
    binding: TurnBinding,
    graph: ArtifactGraph,
    slots: Vec<SlotRecord>,
}

impl CanonicalWebArtifact {
    fn private_constructor_for_internal() -> Self {
        unreachable!("producer-owned constructor")
    }
}

pub(super) struct AcceptedPolishArtifact {
    binding: TurnBinding,
    graph: ArtifactGraph,
}

pub(super) struct TypedMailArtifact {
    binding: TurnBinding,
    graph: ArtifactGraph,
}

pub(super) struct TypedAttachmentArtifact {
    binding: TurnBinding,
    graph: ArtifactGraph,
}

pub(super) struct LegacyAssistantArtifact {
    binding: TurnBinding,
    graph: ArtifactGraph,
    terminal_text_hmac: [u8; 32],
}

pub(super) struct NoMentionArtifact {
    binding: TurnBinding,
    graph: ArtifactGraph,
    reason: ClosedNoMentionReason,
}

impl fmt::Debug for CurrentUserMentionArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CurrentUserMentionArtifact(<sealed>)")
    }
}

macro_rules! sealed_debug {
    ($($name:ident),+ $(,)?) => {
        $(impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<sealed>)"))
            }
        })+
    };
}

sealed_debug!(
    CanonicalWebArtifact,
    AcceptedPolishArtifact,
    TypedMailArtifact,
    TypedAttachmentArtifact,
    LegacyAssistantArtifact,
    NoMentionArtifact,
);

impl CurrentUserMentionArtifact {
    #[cfg(test)]
    pub(super) fn graph(&self) -> &ArtifactGraph {
        &self.graph
    }

    #[cfg(test)]
    pub(super) fn empty_for_test() -> Self {
        Self {
            binding: None,
            input: UserAuthoredText::new(""),
            graph: ArtifactGraph::default(),
        }
    }
}

macro_rules! graph_accessor {
    ($($name:ident),+ $(,)?) => {
        $(impl $name {
            pub(super) fn graph(&self) -> &ArtifactGraph {
                &self.graph
            }
        })+
    };
}

graph_accessor!(
    CanonicalWebArtifact,
    AcceptedPolishArtifact,
    TypedMailArtifact,
    TypedAttachmentArtifact,
    LegacyAssistantArtifact,
    NoMentionArtifact,
);

pub(super) enum OutputArtifact {
    CanonicalWeb {
        canonical: CanonicalWebArtifact,
        polish: Option<AcceptedPolishArtifact>,
    },
    TypedMail {
        mail: TypedMailArtifact,
        polish: Option<AcceptedPolishArtifact>,
    },
    TypedAttachment(TypedAttachmentArtifact),
    LegacyAssistant(LegacyAssistantArtifact),
    NoMention(NoMentionArtifact),
}

impl fmt::Debug for OutputArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OutputArtifact(<sealed>)")
    }
}

impl OutputArtifact {
    fn matches(&self, binding: &TurnBinding) -> bool {
        let matches = |candidate: &TurnBinding| candidate == binding;
        match self {
            Self::CanonicalWeb { canonical, polish } => {
                matches(&canonical.binding)
                    && polish
                        .as_ref()
                        .is_none_or(|polish| matches(&polish.binding))
            }
            Self::TypedMail { mail, polish } => {
                matches(&mail.binding)
                    && polish
                        .as_ref()
                        .is_none_or(|polish| matches(&polish.binding))
            }
            Self::TypedAttachment(attachment) => matches(&attachment.binding),
            Self::LegacyAssistant(assistant) => matches(&assistant.binding),
            Self::NoMention(no_mention) => matches(&no_mention.binding),
        }
    }

    fn graph(&self) -> ArtifactGraph {
        let mut graph = match self {
            Self::CanonicalWeb { canonical, .. } => canonical.graph.clone(),
            Self::TypedMail { mail, .. } => mail.graph.clone(),
            Self::TypedAttachment(attachment) => attachment.graph.clone(),
            Self::LegacyAssistant(assistant) => assistant.graph.clone(),
            Self::NoMention(no_mention) => no_mention.graph.clone(),
        };
        match self {
            Self::CanonicalWeb {
                polish: Some(polish),
                ..
            }
            | Self::TypedMail {
                polish: Some(polish),
                ..
            } => graph.extend(polish.graph.clone()),
            _ => {}
        }
        graph
    }
}

pub(crate) struct CompletedTurnArtifacts {
    binding: TurnBinding,
    input: UserAuthoredText,
    current_user: CurrentUserMentionArtifact,
    output: OutputArtifact,
    terminal: TurnCompletion,
    artifact_digest: [u8; 32],
    graph: ArtifactGraph,
}

impl fmt::Debug for CompletedTurnArtifacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletedTurnArtifacts")
            .field("binding", &self.binding)
            .field("terminal", &self.terminal)
            .field("mention_count", &self.graph.mentions.len())
            .finish()
    }
}

impl CompletedTurnArtifacts {
    pub(super) fn graph(&self) -> &ArtifactGraph {
        &self.graph
    }

    pub(super) fn artifact_digest(&self) -> [u8; 32] {
        self.artifact_digest
    }

    pub(super) fn into_graph(self) -> ArtifactGraph {
        self.graph
    }

    pub(super) fn into_record_parts(self) -> (String, String, Vec<u8>, ArtifactGraph) {
        (
            self.binding.turn_id.as_uuid().to_string(),
            self.binding.session_id.as_str().to_string(),
            self.input.into_string().into_bytes(),
            self.graph,
        )
    }
}

pub(super) fn capture_current_user(
    witness: CurrentInputWitness,
    input: UserAuthoredText,
    extraction: &Extraction,
    custody: &CryptoCustody,
    now_ms: i64,
) -> Result<CurrentUserMentionArtifact, CaptureError> {
    if input.as_str().len() > super::extraction::MAX_MESSAGE_BYTES || now_ms < 0 {
        return Err(CaptureError::MalformedArtifact);
    }
    let mut graph = ArtifactGraph::default();
    let alias_referent_id = (extraction.alias
        && extraction.spans.len() == 2
        && extraction.spans.iter().all(|span| {
            span.sensitivity == MentionSensitivity::Public
                && span.entity_kind != EntityKind::Unknown
        })
        && extraction.spans[0].entity_kind == extraction.spans[1].entity_kind)
        .then(|| Uuid::new_v4().to_string());
    for span in &extraction.spans {
        let range = span.span.start_utf8()..span.span.end_utf8();
        let Some(display) = input.as_str().get(range.clone()) else {
            return Err(CaptureError::BindingMismatch);
        };
        if display != span.display || display.len() > super::extraction::MAX_SPAN_BYTES {
            return Err(CaptureError::BindingMismatch);
        }
        let mention_id = Uuid::new_v4().to_string();
        let referent_id = alias_referent_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let representation = match span.sensitivity {
            MentionSensitivity::Public => {
                if span.entity_kind != EntityKind::Unknown {
                    MentionRepresentation::PublicVisible {
                        display: display.to_string(),
                        normalized: span.normalized.clone(),
                    }
                } else {
                    MentionRepresentation::Restricted {
                        span_hmac: hmac(
                            custody,
                            &witness.binding,
                            "restricted",
                            display.as_bytes(),
                        )?,
                    }
                }
            }
            MentionSensitivity::Sensitive => MentionRepresentation::Opaque {
                fingerprint: hmac(custody, &witness.binding, "sensitive", display.as_bytes())?,
            },
            MentionSensitivity::Private | MentionSensitivity::Unknown => {
                MentionRepresentation::Restricted {
                    span_hmac: hmac(custody, &witness.binding, "restricted", display.as_bytes())?,
                }
            }
        };
        let (visibility, sensitivity) = match span.sensitivity {
            MentionSensitivity::Public if span.entity_kind != EntityKind::Unknown => {
                ("provider_safe", "public")
            }
            MentionSensitivity::Private => ("local_only", "private"),
            MentionSensitivity::Sensitive => ("local_only", "sensitive"),
            _ => ("local_only", "unknown"),
        };
        let anchor_id = Uuid::new_v4().to_string();
        let anchor_hmac = hmac(custody, &witness.binding, "user_anchor", display.as_bytes())?;
        graph.mentions.push(MentionArtifact {
            mention_id: mention_id.clone(),
            referent_id,
            turn_id: witness.binding.turn_id.as_uuid().to_string(),
            session_id: witness.binding.session_id.as_str().to_string(),
            canonical_parent_mention_id: None,
            entity_kind: span.entity_kind.as_str().to_string(),
            provenance: "user_authored".into(),
            assistant_lineage: None,
            producer: "resolver_user_input".into(),
            visibility: visibility.into(),
            sensitivity: sensitivity.into(),
            direct_user: true,
            untrusted_evidence: false,
            origin_ref_hmac: None,
            mail_body_origin: None,
            representation,
            created_at_ms: now_ms,
            expires_at_ms: now_ms + 1_800_000,
            hmac_key_version: 1,
            encryption_key_version: Some(1),
        });
        graph.anchors.push(AnchorArtifact {
            anchor_id,
            mention_id,
            turn_id: witness.binding.turn_id.as_uuid().to_string(),
            kind: AnchorKind::Visible,
            display_class: DisplayClass::UserInput,
            ordinal: graph.anchors.len() as i64,
            start_utf8: Some(span.span.start_utf8() as i64),
            end_utf8: Some(span.span.end_utf8() as i64),
            visible_span_hmac: Some(anchor_hmac),
            opaque_anchor_hmac: None,
            hmac_key_version: 1,
            created_at_ms: now_ms,
        });
    }
    Ok(CurrentUserMentionArtifact {
        binding: Some(witness.binding),
        input,
        graph,
    })
}

pub(super) fn finish_canonical_web(
    witness: CanonicalRendererWitness,
    rendered: FinalOutputText,
    slots: Vec<CanonicalMentionSlot>,
    claims: Vec<CanonicalClaim>,
    status: CanonicalOutcomeStatus,
    complete: bool,
    custody: &CryptoCustody,
    now_ms: i64,
) -> Result<CanonicalWebArtifact, CaptureError> {
    if status != CanonicalOutcomeStatus::Verified || !complete || now_ms < 0 {
        return Err(CaptureError::PolicyRejected);
    }
    let mut graph = ArtifactGraph::default();
    let mut records = Vec::new();
    for slot in slots {
        let claim = claims
            .iter()
            .find(|claim| claim.id == slot.claim_id)
            .ok_or(CaptureError::IncompleteLineage)?;
        if slot.sources.is_empty()
            || slot.entity_kind == EntityKind::Unknown
            || slot.spans.is_empty()
            || !slot.spans.iter().all(|range| {
                rendered.0.as_bytes().get(range.clone()) == Some(slot.display.as_bytes())
            })
        {
            return Err(CaptureError::PolicyRejected);
        }
        let mut source_signature = Vec::new();
        let mut independent = std::collections::BTreeSet::new();
        for source in &slot.sources {
            if !claim
                .evidence_ids
                .iter()
                .any(|id| id == &source.evidence_id)
                || !public_url(&source.final_url)
                || source.source_identity.as_str().is_empty()
                || !rendered.0.contains(source.final_url.as_str())
                || (source.authority == SourceAuthority::FirstParty
                    && !source.page_owner_entity_bound)
            {
                return Err(CaptureError::PolicyRejected);
            }
            independent.insert(source.source_identity.as_str().to_string());
            let evidence_hmac = hmac(
                custody,
                &witness.binding,
                "web_evidence",
                source.evidence_id.as_str().as_bytes(),
            )?;
            source_signature.push((evidence_hmac, source.source_identity.as_str().to_string()));
        }
        let needs_independent = slot
            .sources
            .iter()
            .all(|source| source.authority == SourceAuthority::Other);
        if needs_independent && independent.len() < 2 {
            return Err(CaptureError::PolicyRejected);
        }
        let mention_id = Uuid::new_v4().to_string();
        let referent_id = Uuid::new_v4().to_string();
        graph.mentions.push(MentionArtifact {
            mention_id: mention_id.clone(),
            referent_id,
            turn_id: witness.binding.turn_id.as_uuid().to_string(),
            session_id: witness.binding.session_id.as_str().to_string(),
            canonical_parent_mention_id: None,
            entity_kind: slot.entity_kind.as_str().into(),
            provenance: "web_evidence".into(),
            assistant_lineage: None,
            producer: "canonical_web".into(),
            visibility: "provider_safe".into(),
            sensitivity: "public".into(),
            direct_user: false,
            untrusted_evidence: false,
            origin_ref_hmac: Some(source_signature[0].0),
            mail_body_origin: None,
            representation: MentionRepresentation::PublicVisible {
                display: slot.display.clone(),
                normalized: slot.normalized.clone(),
            },
            created_at_ms: now_ms,
            expires_at_ms: now_ms + 1_800_000,
            hmac_key_version: 1,
            encryption_key_version: Some(1),
        });
        for (ordinal, range) in slot.spans.iter().enumerate() {
            let anchor_id = Uuid::new_v4().to_string();
            graph.anchors.push(AnchorArtifact {
                anchor_id: anchor_id.clone(),
                mention_id: mention_id.clone(),
                turn_id: witness.binding.turn_id.as_uuid().to_string(),
                kind: AnchorKind::Visible,
                display_class: DisplayClass::Canonical,
                ordinal: ordinal as i64,
                start_utf8: Some(range.start as i64),
                end_utf8: Some(range.end as i64),
                visible_span_hmac: Some(hmac(
                    custody,
                    &witness.binding,
                    "canonical_anchor",
                    slot.display.as_bytes(),
                )?),
                opaque_anchor_hmac: None,
                hmac_key_version: 1,
                created_at_ms: now_ms,
            });
        }
        let canonical_anchor_id = graph
            .anchors
            .iter()
            .rev()
            .find(|anchor| anchor.mention_id == mention_id)
            .expect("canonical anchor inserted")
            .anchor_id
            .clone();
        for (source_ordinal, source) in slot.sources.iter().enumerate() {
            let evidence_hmac = hmac(
                custody,
                &witness.binding,
                "web_evidence",
                source.evidence_id.as_str().as_bytes(),
            )?;
            let source_hmac = hmac(
                custody,
                &witness.binding,
                "source_identity",
                source.source_identity.as_str().as_bytes(),
            )?;
            let url_hmac = hmac(
                custody,
                &witness.binding,
                "public_url",
                source.final_url.as_str().as_bytes(),
            )?;
            graph.web_mappings.push(WebMappingArtifact {
                mapping_id: Uuid::new_v4().to_string(),
                mention_id: mention_id.clone(),
                canonical_anchor_id: canonical_anchor_id.clone(),
                source_ordinal: source_ordinal as i64,
                evidence_id_hmac: evidence_hmac,
                source_identity: source.source_identity.as_str().into(),
                source_identity_hmac: source_hmac,
                public_url: source.final_url.as_str().into(),
                public_url_hmac: url_hmac,
                authority: authority_str(source.authority).into(),
                network_policy_version: 1,
                validated_at_ms: now_ms,
                encryption_key_version: 1,
                hmac_key_version: 1,
            });
        }
        records.push(SlotRecord {
            mention_id,
            display: slot.display,
            normalized: slot.normalized,
            entity_kind: slot.entity_kind,
            claim_id: slot.claim_id,
            source_signature,
        });
    }
    Ok(CanonicalWebArtifact {
        binding: witness.binding,
        graph,
        slots: records,
    })
}

pub(super) fn accept_polish(
    witness: AcceptedPolishWitness,
    canonical: &CanonicalWebArtifact,
    polished: FinalOutputText,
    slots: Vec<PolishSlot>,
    custody: &CryptoCustody,
    now_ms: i64,
) -> Result<AcceptedPolishArtifact, CaptureError> {
    if witness.binding != canonical.binding {
        return Err(CaptureError::BindingMismatch);
    }
    if slots.len() != canonical.slots.len() {
        return Err(CaptureError::PolicyRejected);
    }
    let mut graph = ArtifactGraph::default();
    for (position, slot) in slots.iter().enumerate() {
        let expected = canonical
            .slots
            .get(position)
            .ok_or(CaptureError::PolicyRejected)?;
        if slot.index != position
            || slot.display != expected.display
            || slot.normalized != expected.normalized
            || slot.entity_kind != expected.entity_kind
            || slot.claim_id != expected.claim_id
            || slot.source_signature != expected.source_signature
            || !slot.spans.iter().all(|range| {
                polished.0.as_bytes().get(range.clone()) == Some(slot.display.as_bytes())
            })
        {
            return Err(CaptureError::PolicyRejected);
        }
        let mention_id = Uuid::new_v4().to_string();
        let canonical_parent = &expected.mention_id;
        graph.mentions.push(MentionArtifact {
            mention_id: mention_id.clone(),
            referent_id: canonical_parent.clone(),
            turn_id: witness.binding.turn_id.as_uuid().to_string(),
            session_id: witness.binding.session_id.as_str().to_string(),
            canonical_parent_mention_id: Some(canonical_parent.clone()),
            entity_kind: slot.entity_kind.as_str().into(),
            provenance: "web_evidence".into(),
            assistant_lineage: None,
            producer: "accepted_polish".into(),
            visibility: "provider_safe".into(),
            sensitivity: "public".into(),
            direct_user: false,
            untrusted_evidence: false,
            origin_ref_hmac: Some(slot.source_signature[0].0),
            mail_body_origin: None,
            representation: MentionRepresentation::PublicVisible {
                display: slot.display.clone(),
                normalized: slot.normalized.clone(),
            },
            created_at_ms: now_ms,
            expires_at_ms: now_ms + 1_800_000,
            hmac_key_version: 1,
            encryption_key_version: Some(1),
        });
        graph.derivations.push(DerivationArtifact {
            derivation_id: Uuid::new_v4().to_string(),
            derived_mention_id: mention_id.clone(),
            parent_mention_id: canonical_parent.clone(),
            kind: DerivationKind::AcceptedPolishOf,
            parent_ordinal: position as i64,
            created_at_ms: now_ms,
        });
        for (ordinal, range) in slot.spans.iter().enumerate() {
            graph.anchors.push(AnchorArtifact {
                anchor_id: Uuid::new_v4().to_string(),
                mention_id: mention_id.clone(),
                turn_id: witness.binding.turn_id.as_uuid().to_string(),
                kind: AnchorKind::Visible,
                display_class: DisplayClass::AcceptedPolish,
                ordinal: ordinal as i64,
                start_utf8: Some(range.start as i64),
                end_utf8: Some(range.end as i64),
                visible_span_hmac: Some(hmac(
                    custody,
                    &witness.binding,
                    "polished_anchor",
                    slot.display.as_bytes(),
                )?),
                opaque_anchor_hmac: None,
                hmac_key_version: 1,
                created_at_ms: now_ms,
            });
        }
    }
    Ok(AcceptedPolishArtifact {
        binding: witness.binding,
        graph,
    })
}

pub(super) fn finish_typed_mail(
    witness: CanonicalMailRendererWitness,
    rendered: FinalOutputText,
    slots: Vec<TypedMailSlot>,
    custody: &CryptoCustody,
    now_ms: i64,
) -> Result<TypedMailArtifact, CaptureError> {
    let mut graph = ArtifactGraph::default();
    for slot in slots {
        if !safe_private_source_slot(&slot.display, slot.entity_kind)
            || slot.evidence.body_state != BodyState::Readable
            || slot.evidence.body_origin == BodyOrigin::Unavailable
            || !slot.evidence.body.contains(&slot.display)
            || !exact_ranges(&rendered.0, &slot.display, &slot.spans)
        {
            continue;
        }
        let mention_id = Uuid::new_v4().to_string();
        let evidence_hmac = hmac(
            custody,
            &witness.binding,
            "mail_evidence",
            slot.evidence.evidence_id.as_str().as_bytes(),
        )?;
        graph.mentions.push(MentionArtifact {
            mention_id: mention_id.clone(),
            referent_id: Uuid::new_v4().to_string(),
            turn_id: witness.binding.turn_id.as_uuid().to_string(),
            session_id: witness.binding.session_id.as_str().to_string(),
            canonical_parent_mention_id: None,
            entity_kind: slot.entity_kind.as_str().into(),
            provenance: "mail_evidence".into(),
            assistant_lineage: None,
            producer: "typed_mail".into(),
            visibility: "confirmation_only".into(),
            sensitivity: "public".into(),
            direct_user: false,
            untrusted_evidence: true,
            origin_ref_hmac: Some(evidence_hmac),
            mail_body_origin: Some(body_origin_str(slot.evidence.body_origin).into()),
            representation: MentionRepresentation::Restricted {
                span_hmac: hmac(
                    custody,
                    &witness.binding,
                    "mail_projection",
                    slot.display.as_bytes(),
                )?,
            },
            created_at_ms: now_ms,
            expires_at_ms: now_ms + 1_800_000,
            hmac_key_version: 1,
            encryption_key_version: None,
        });
        for (ordinal, range) in slot.spans.iter().enumerate() {
            graph.anchors.push(AnchorArtifact {
                anchor_id: Uuid::new_v4().to_string(),
                mention_id: mention_id.clone(),
                turn_id: witness.binding.turn_id.as_uuid().to_string(),
                kind: AnchorKind::Visible,
                display_class: DisplayClass::RestrictedOutput,
                ordinal: ordinal as i64,
                start_utf8: Some(range.start as i64),
                end_utf8: Some(range.end as i64),
                visible_span_hmac: Some(hmac(
                    custody,
                    &witness.binding,
                    "mail_anchor",
                    slot.display.as_bytes(),
                )?),
                opaque_anchor_hmac: None,
                hmac_key_version: 1,
                created_at_ms: now_ms,
            });
        }
    }
    Ok(TypedMailArtifact {
        binding: witness.binding,
        graph,
    })
}

pub(super) fn finish_typed_attachment(
    witness: AttachmentExtractionWitness,
    rendered: FinalOutputText,
    slots: Vec<TypedAttachmentSlot>,
    extraction: SupportedTypedExtraction,
    custody: &CryptoCustody,
    now_ms: i64,
) -> Result<TypedAttachmentArtifact, CaptureError> {
    if !extraction.supported || extraction.attachment_ref.trim().is_empty() {
        return Ok(TypedAttachmentArtifact {
            binding: witness.binding,
            graph: ArtifactGraph::default(),
        });
    }
    let mut graph = ArtifactGraph::default();
    for slot in slots {
        if !safe_private_source_slot(&slot.display, slot.entity_kind)
            || !extraction.text.contains(&slot.display)
            || !exact_ranges(&rendered.0, &slot.display, &slot.spans)
        {
            continue;
        }
        let mention_id = Uuid::new_v4().to_string();
        let attachment_hmac = hmac(
            custody,
            &witness.binding,
            "attachment_identity",
            extraction.attachment_ref.as_bytes(),
        )?;
        graph.mentions.push(MentionArtifact {
            mention_id: mention_id.clone(),
            referent_id: Uuid::new_v4().to_string(),
            turn_id: witness.binding.turn_id.as_uuid().to_string(),
            session_id: witness.binding.session_id.as_str().to_string(),
            canonical_parent_mention_id: None,
            entity_kind: slot.entity_kind.as_str().into(),
            provenance: "attachment_evidence".into(),
            assistant_lineage: None,
            producer: "typed_attachment".into(),
            visibility: "confirmation_only".into(),
            sensitivity: "public".into(),
            direct_user: false,
            untrusted_evidence: true,
            origin_ref_hmac: Some(attachment_hmac),
            mail_body_origin: None,
            representation: MentionRepresentation::Restricted {
                span_hmac: hmac(
                    custody,
                    &witness.binding,
                    "attachment_projection",
                    slot.display.as_bytes(),
                )?,
            },
            created_at_ms: now_ms,
            expires_at_ms: now_ms + 1_800_000,
            hmac_key_version: 1,
            encryption_key_version: None,
        });
        for (ordinal, range) in slot.spans.iter().enumerate() {
            graph.anchors.push(AnchorArtifact {
                anchor_id: Uuid::new_v4().to_string(),
                mention_id: mention_id.clone(),
                turn_id: witness.binding.turn_id.as_uuid().to_string(),
                kind: AnchorKind::Visible,
                display_class: DisplayClass::RestrictedOutput,
                ordinal: ordinal as i64,
                start_utf8: Some(range.start as i64),
                end_utf8: Some(range.end as i64),
                visible_span_hmac: Some(hmac(
                    custody,
                    &witness.binding,
                    "attachment_anchor",
                    slot.display.as_bytes(),
                )?),
                opaque_anchor_hmac: None,
                hmac_key_version: 1,
                created_at_ms: now_ms,
            });
        }
    }
    Ok(TypedAttachmentArtifact {
        binding: witness.binding,
        graph,
    })
}

pub(super) fn finish_legacy_assistant(
    witness: AcceptedLegacyTerminalWitness,
    final_text: FinalOutputText,
    custody: &CryptoCustody,
    _now_ms: i64,
) -> LegacyAssistantArtifact {
    let terminal_text_hmac = hmac(
        custody,
        &witness.binding,
        "legacy_terminal_text",
        final_text.0.as_bytes(),
    )
    .unwrap_or([0; 32]);
    LegacyAssistantArtifact {
        binding: witness.binding,
        graph: ArtifactGraph::default(),
        terminal_text_hmac,
    }
}

pub(super) fn finish_no_mention(
    witness: DeterministicTerminalWitness,
    reason: ClosedNoMentionReason,
) -> NoMentionArtifact {
    NoMentionArtifact {
        binding: witness.binding,
        graph: ArtifactGraph::default(),
        reason,
    }
}

fn hmac(
    custody: &CryptoCustody,
    binding: &TurnBinding,
    purpose: &str,
    value: &[u8],
) -> Result<[u8; 32], CaptureError> {
    custody
        .hmac(
            &AadBinding::new(
                binding.turn_id.as_uuid().to_string(),
                binding.session_id.as_str(),
                purpose,
            ),
            1,
            value,
        )
        .map_err(|_| CaptureError::UnsupportedProducerState)
}

fn normalize_term(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn exact_ranges(rendered: &str, display: &str, ranges: &[Range<usize>]) -> bool {
    !ranges.is_empty()
        && ranges
            .iter()
            .all(|range| rendered.as_bytes().get(range.clone()) == Some(display.as_bytes()))
}

fn public_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && !url.host_str().is_some_and(|host| {
            let host = host.to_ascii_lowercase();
            host == "localhost"
                || host.ends_with(".local")
                || host.starts_with("127.")
                || host.starts_with("10.")
                || host.starts_with("192.168.")
        })
}

fn authority_str(authority: SourceAuthority) -> &'static str {
    match authority {
        SourceAuthority::FirstParty => "first_party",
        SourceAuthority::AuthoritativeReference => "authoritative_reference",
        SourceAuthority::Other => "other",
    }
}

fn safe_private_source_slot(display: &str, kind: EntityKind) -> bool {
    if display.len() > 256
        || !matches!(
            kind,
            EntityKind::Product | EntityKind::TechnicalStandard | EntityKind::Organization
        )
    {
        return false;
    }
    let lower = display.to_ascii_lowercase();
    let starts_with_private_identifier = [
        "serial",
        "order",
        "tracking",
        "credential",
        "password",
        "token",
        "secret",
        "iban",
        "invoice",
    ]
    .iter()
    .any(|prefix| {
        lower
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || !rest.as_bytes()[0].is_ascii_alphanumeric())
    });
    !lower.contains("http://")
        && !lower.contains("https://")
        && !starts_with_private_identifier
        && !lower.starts_with("sk-")
}

fn body_origin_str(origin: BodyOrigin) -> &'static str {
    match origin {
        BodyOrigin::LocalEmlx => "local_emlx",
        BodyOrigin::MailAutomation => "mail_automation",
        BodyOrigin::Unavailable => "unavailable",
    }
}

fn graph_is_acyclic(graph: &ArtifactGraph) -> bool {
    let mut edges = std::collections::HashMap::<&str, &str>::new();
    for derivation in &graph.derivations {
        if derivation.derived_mention_id == derivation.parent_mention_id
            || edges
                .insert(
                    &derivation.derived_mention_id,
                    &derivation.parent_mention_id,
                )
                .is_some()
        {
            return false;
        }
    }
    for start in edges.keys().copied() {
        let mut seen = std::collections::HashSet::new();
        let mut current = start;
        while let Some(next) = edges.get(current).copied() {
            if !seen.insert(current) {
                return false;
            }
            current = next;
        }
    }
    true
}

impl ArtifactGraph {
    fn extend(&mut self, other: ArtifactGraph) {
        self.mentions.extend(other.mentions);
        self.derivations.extend(other.derivations);
        self.anchors.extend(other.anchors);
        self.web_mappings.extend(other.web_mappings);
    }

    fn digest(&self) -> [u8; 32] {
        let mut encoded = Vec::new();
        for part in self.canonical_parts() {
            encoded.extend_from_slice(&(part.len() as u32).to_be_bytes());
            encoded.extend_from_slice(&part);
        }
        sha2::Sha256::digest(encoded).into()
    }
}

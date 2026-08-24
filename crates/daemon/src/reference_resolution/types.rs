#![allow(dead_code)]

use crate::evidence::{BodyOrigin, EvidenceId, SourceAuthority, SourceIdentity};
use chrono::{DateTime, Utc};
use std::fmt;
use uuid::Uuid;

pub(crate) use crate::evidence::EvidenceOrigin as TurnOrigin;

fn redacted_debug(formatter: &mut fmt::Formatter<'_>, type_name: &'static str) -> fmt::Result {
    formatter
        .debug_struct(type_name)
        .field("value", &"<redacted>")
        .finish()
}

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub(crate) struct $name(Uuid);

        impl $name {
            pub(crate) fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub(crate) const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub(crate) const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                redacted_debug(formatter, stringify!($name))
            }
        }
    };
}

uuid_id!(TurnId);
uuid_id!(MentionId);
uuid_id!(ConfirmationId);
uuid_id!(AuthorizationId);
uuid_id!(ProviderAttemptId);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdValidationError {
    Empty,
    ControlCharacter,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SessionId(String);

impl SessionId {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, IdValidationError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(IdValidationError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(IdValidationError::ControlCharacter);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_debug(formatter, "SessionId")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct UserAuthoredText(String);

impl UserAuthoredText {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for UserAuthoredText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_debug(formatter, "UserAuthoredText")
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct NormalizedPublicText(String);

impl NormalizedPublicText {
    pub(super) fn new(value: impl Into<String>) -> Result<Self, IdValidationError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(IdValidationError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(IdValidationError::ControlCharacter);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NormalizedPublicText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_debug(formatter, "NormalizedPublicText")
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct ProposedPublicTerm(String);

impl ProposedPublicTerm {
    pub(super) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Debug for ProposedPublicTerm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_debug(formatter, "ProposedPublicTerm")
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct PublicUrlReference(String);

impl PublicUrlReference {
    pub(super) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Debug for PublicUrlReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_debug(formatter, "PublicUrlReference")
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct OpaqueAttachmentReference(String);

impl OpaqueAttachmentReference {
    pub(super) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Debug for OpaqueAttachmentReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_debug(formatter, "OpaqueAttachmentReference")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MentionDigest([u8; 32]);

impl MentionDigest {
    pub(crate) const fn zero() -> Self {
        Self([0; 32])
    }
}

impl fmt::Debug for MentionDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MentionDigest(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CurrentTurnSpan {
    start_utf8: usize,
    end_utf8: usize,
}

impl CurrentTurnSpan {
    pub(crate) fn new(start_utf8: usize, end_utf8: usize) -> Option<Self> {
        (start_utf8 <= end_utf8).then_some(Self {
            start_utf8,
            end_utf8,
        })
    }

    pub(crate) const fn start_utf8(self) -> usize {
        self.start_utf8
    }

    pub(crate) const fn end_utf8(self) -> usize {
        self.end_utf8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginalRequestScope {
    NotApplicable,
    SupportedStage9,
    ReferenceBearing,
    MixedMailWeb,
    UnsafeReferenceBearing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EntityKind {
    Person,
    Organization,
    Place,
    Product,
    TechnicalStandard,
    DocumentTitle,
    PublicUrl,
    Unknown,
}

impl EntityKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Place => "place",
            Self::Product => "product",
            Self::TechnicalStandard => "technical_standard",
            Self::DocumentTitle => "document_title",
            Self::PublicUrl => "public_url",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "person" => Self::Person,
            "organization" => Self::Organization,
            "place" => Self::Place,
            "product" => Self::Product,
            "technical_standard" => Self::TechnicalStandard,
            "document_title" => Self::DocumentTitle,
            "public_url" => Self::PublicUrl,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrammaticalNumber {
    Singular,
    Plural,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceExpressionKind {
    Pronoun,
    Demonstrative,
    GenericNoun,
    NamedReuse,
    Comparison,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceExpression {
    kind: ReferenceExpressionKind,
    span: CurrentTurnSpan,
    compatible_kinds: Vec<EntityKind>,
    grammatical_number: GrammaticalNumber,
}

impl ReferenceExpression {
    pub(crate) fn new(
        kind: ReferenceExpressionKind,
        span: CurrentTurnSpan,
        compatible_kinds: Vec<EntityKind>,
        grammatical_number: GrammaticalNumber,
    ) -> Self {
        Self {
            kind,
            span,
            compatible_kinds,
            grammatical_number,
        }
    }

    pub(crate) const fn kind(&self) -> ReferenceExpressionKind {
        self.kind
    }

    pub(crate) const fn span(&self) -> CurrentTurnSpan {
        self.span
    }

    pub(crate) fn compatible_kinds(&self) -> &[EntityKind] {
        &self.compatible_kinds
    }

    pub(crate) const fn grammatical_number(&self) -> GrammaticalNumber {
        self.grammatical_number
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtractedSpanKind {
    SensitiveIdentifier,
    HttpUrl,
    NumberedTechnicalStandard,
    MakeModelProduct,
    DocumentTitle,
    Organization,
    Person,
    Place,
    BacktickedUnknown,
    QuotedUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedMentionSpan {
    pub(crate) span: CurrentTurnSpan,
    pub(crate) display: String,
    pub(crate) normalized: String,
    pub(crate) kind: ExtractedSpanKind,
    pub(crate) entity_kind: EntityKind,
    pub(crate) sensitivity: MentionSensitivity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LedgerProvenance {
    PriorUser,
    CanonicalWeb,
    AcceptedPolish,
    Assistant,
    Mail,
    Attachment,
    Unknown,
}

impl LedgerProvenance {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PriorUser => "prior_user",
            Self::CanonicalWeb => "canonical_web",
            Self::AcceptedPolish => "accepted_polish",
            Self::Assistant => "assistant",
            Self::Mail => "mail",
            Self::Attachment => "attachment",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LedgerCandidate {
    pub(crate) mention_id: MentionId,
    pub(crate) referent_id: String,
    pub(crate) entity_kind: EntityKind,
    pub(crate) display: Option<String>,
    pub(crate) normalized: Option<String>,
    pub(crate) provenance: LedgerProvenance,
    pub(crate) visibility: MentionVisibility,
    pub(crate) sensitivity: MentionSensitivity,
    pub(crate) introduced_sequence: i64,
    pub(crate) created_at_ms: i64,
    pub(crate) expires_at_ms: i64,
    pub(crate) age_turns: u8,
    pub(crate) age_minutes: u32,
    pub(crate) canonical_mapping_intact: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateCompatibility {
    Compatible,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateRecency {
    CurrentTurn,
    Recent,
    OutsideWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateEligibility {
    Eligible,
    ConfirmationRequired,
    Denied,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolutionReason {
    MissingReferent,
    AmbiguousCandidates,
    ConfirmationRequired,
    PrivateOrSensitive,
    ExpiredMention,
    UnsupportedRequest,
    InvalidScope,
    ResolverUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceCandidate {
    mention_id: MentionId,
    compatibility: CandidateCompatibility,
    recency: CandidateRecency,
    eligibility: CandidateEligibility,
    denial_reasons: Vec<ResolutionReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolutionConfidence {
    ExactCurrent,
    UniqueRecent,
    Confirmed,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MentionVisibility {
    ProviderSafe,
    LocalOnly,
    ConfirmationOnly,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MentionSensitivity {
    Public,
    Private,
    Sensitive,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssistantLineage {
    Canonical,
    AcceptedPolish,
    RejectedPolish,
    Legacy,
    Unknown,
}

#[derive(PartialEq, Eq)]
pub(crate) enum MentionText {
    PublicVisible {
        display: String,
        normalized: NormalizedPublicText,
    },
    Restricted {
        span_hmac: MentionDigest,
        kind_hint: EntityKind,
    },
    Opaque {
        fingerprint: MentionDigest,
        kind_hint: EntityKind,
    },
}

impl MentionText {
    pub(super) fn public(
        display: impl Into<String>,
        normalized: impl Into<String>,
    ) -> Result<Self, IdValidationError> {
        Ok(Self::PublicVisible {
            display: display.into(),
            normalized: NormalizedPublicText::new(normalized)?,
        })
    }
}

impl fmt::Debug for MentionText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PublicVisible { .. } => redacted_debug(formatter, "MentionText::PublicVisible"),
            Self::Restricted { kind_hint, .. } => formatter
                .debug_struct("MentionText::Restricted")
                .field("span_hmac", &"<redacted>")
                .field("kind_hint", kind_hint)
                .finish(),
            Self::Opaque { kind_hint, .. } => formatter
                .debug_struct("MentionText::Opaque")
                .field("fingerprint", &"<redacted>")
                .field("kind_hint", kind_hint)
                .finish(),
        }
    }
}

#[derive(PartialEq, Eq)]
pub(crate) enum MentionProvenance {
    UserAuthored,
    AssistantAuthored {
        lineage: AssistantLineage,
    },
    MailEvidence {
        evidence_id: EvidenceId,
        body_origin: BodyOrigin,
    },
    WebEvidence {
        evidence_id: EvidenceId,
        source_identity: SourceIdentity,
        authority: SourceAuthority,
        public_source: PublicUrlReference,
        canonical: bool,
    },
    AttachmentEvidence {
        attachment_ref: OpaqueAttachmentReference,
    },
    Unknown,
}

impl fmt::Debug for MentionProvenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::UserAuthored => "user_authored",
            Self::AssistantAuthored { .. } => "assistant_authored",
            Self::MailEvidence { .. } => "mail_evidence",
            Self::WebEvidence { .. } => "web_evidence",
            Self::AttachmentEvidence { .. } => "attachment_evidence",
            Self::Unknown => "unknown",
        };
        formatter
            .debug_struct("MentionProvenance")
            .field("class", &label)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MentionAnchorKind {
    Visible,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MentionDisplayClass {
    UserInput,
    Canonical,
    AcceptedPolish,
    RestrictedOutput,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MentionAnchor {
    kind: MentionAnchorKind,
    display_class: MentionDisplayClass,
    span: Option<CurrentTurnSpan>,
}

#[derive(PartialEq, Eq)]
pub(crate) struct ConversationMention {
    id: MentionId,
    session_id: SessionId,
    introduced_turn_id: TurnId,
    entity_kind: EntityKind,
    text: MentionText,
    provenance: MentionProvenance,
    visibility: MentionVisibility,
    sensitivity: MentionSensitivity,
    directly_user_supplied: bool,
    untrusted_evidence: bool,
    anchor: MentionAnchor,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl fmt::Debug for ConversationMention {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConversationMention")
            .field("id", &self.id)
            .field("entity_kind", &self.entity_kind)
            .field("provenance", &self.provenance)
            .field("visibility", &self.visibility)
            .field("sensitivity", &self.sensitivity)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClarificationAction {
    Confirm,
    Edit,
}

#[derive(PartialEq, Eq)]
pub(crate) struct ClarificationRequest {
    challenge_id: Option<ConfirmationId>,
    safe_entity_label: Option<EntityKind>,
    local_display_proposal: Option<ProposedPublicTerm>,
    expires_at: Option<DateTime<Utc>>,
    allowed_actions: Vec<ClarificationAction>,
    normalized_reason: ResolutionReason,
}

impl fmt::Debug for ClarificationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClarificationRequest")
            .field("challenge_id", &self.challenge_id)
            .field("safe_entity_label", &self.safe_entity_label)
            .field("has_proposal", &self.local_display_proposal.is_some())
            .field("expires_at", &self.expires_at)
            .field("allowed_actions", &self.allowed_actions)
            .field("normalized_reason", &self.normalized_reason)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceOutcomeCode {
    MissingReferent,
    Ambiguous,
    ConfirmationRequired,
    PrivateSourceDenied,
    Expired,
    Unsupported,
    ResolverUnavailable,
}

impl ReferenceOutcomeCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MissingReferent => "missing_referent",
            Self::Ambiguous => "ambiguous",
            Self::ConfirmationRequired => "confirmation_required",
            Self::PrivateSourceDenied => "private_source_denied",
            Self::Expired => "expired",
            Self::Unsupported => "unsupported",
            Self::ResolverUnavailable => "resolver_unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockedPresentation {
    MissingPublicTerm,
    Ambiguous,
    ConfirmationRequired,
    PrivateSourceDenied,
    Expired,
    Unsupported,
    ResolverUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReferenceBlock {
    outcome: ReferenceOutcomeCode,
    presentation: BlockedPresentation,
}

impl ReferenceBlock {
    pub(crate) const fn new(
        outcome: ReferenceOutcomeCode,
        presentation: BlockedPresentation,
    ) -> Self {
        Self {
            outcome,
            presentation,
        }
    }

    pub(crate) const fn outcome(self) -> ReferenceOutcomeCode {
        self.outcome
    }

    pub(crate) const fn presentation(self) -> BlockedPresentation {
        self.presentation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolverFault {
    InvalidInput,
    Unavailable,
    CorruptState,
    InvariantViolation,
    ConflictingRetry,
    Configuration,
    AlreadyConsumed,
}

impl ResolverFault {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::Unavailable => "unavailable",
            Self::CorruptState => "corrupt_state",
            Self::InvariantViolation => "invariant_violation",
            Self::ConflictingRetry => "conflicting_retry",
            Self::Configuration => "configuration",
            Self::AlreadyConsumed => "already_consumed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnCompletion {
    Completed,
    Partial,
    ReferenceBlocked(ReferenceOutcomeCode),
}

#[derive(PartialEq, Eq)]
pub(crate) struct ConfirmationEnvelope {
    challenge_id: ConfirmationId,
    proposed_term: String,
}

impl ConfirmationEnvelope {
    pub(super) fn new(challenge_id: ConfirmationId, proposed_term: impl Into<String>) -> Self {
        Self {
            challenge_id,
            proposed_term: proposed_term.into(),
        }
    }

    pub(crate) const fn challenge_id(&self) -> ConfirmationId {
        self.challenge_id
    }

    pub(crate) fn proposed_term(&self) -> &str {
        &self.proposed_term
    }
}

impl fmt::Debug for ConfirmationEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfirmationEnvelope")
            .field("challenge_id", &self.challenge_id)
            .field("has_proposed_term", &true)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestAction {
    UserText,
    Confirmation,
    EditedUserText,
}

pub(crate) struct ResolveTurn {
    turn_id: TurnId,
    session_id: SessionId,
    origin: TurnOrigin,
    current_input: UserAuthoredText,
    original_scope: OriginalRequestScope,
    confirmation: Option<ConfirmationEnvelope>,
    action: RequestAction,
}

impl ResolveTurn {
    pub(crate) fn new(
        turn_id: TurnId,
        session_id: SessionId,
        origin: TurnOrigin,
        current_input: UserAuthoredText,
        original_scope: OriginalRequestScope,
        confirmation: Option<ConfirmationEnvelope>,
    ) -> Self {
        Self {
            turn_id,
            session_id,
            origin,
            current_input,
            original_scope,
            confirmation,
            action: RequestAction::UserText,
        }
    }

    pub(crate) fn with_action(mut self, action: RequestAction) -> Self {
        self.action = action;
        self
    }

    pub(crate) const fn turn_id(&self) -> TurnId {
        self.turn_id
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub(crate) const fn origin(&self) -> TurnOrigin {
        self.origin
    }

    pub(crate) fn current_input(&self) -> &UserAuthoredText {
        &self.current_input
    }

    pub(crate) const fn scope(&self) -> OriginalRequestScope {
        self.original_scope
    }

    pub(crate) fn confirmation(&self) -> Option<&ConfirmationEnvelope> {
        self.confirmation.as_ref()
    }

    pub(crate) const fn action(&self) -> RequestAction {
        self.action
    }
}

impl fmt::Debug for ResolveTurn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolveTurn")
            .field("turn_id", &self.turn_id)
            .field("session_id", &self.session_id)
            .field("origin", &self.origin)
            .field("current_input", &"<redacted>")
            .field("original_scope", &self.original_scope)
            .field("action", &self.action)
            .field("has_confirmation", &self.confirmation.is_some())
            .finish()
    }
}

pub(crate) enum ResolvedRequestView {
    LiteralCurrentTurn,
    WebReference {
        original_request: UserAuthoredText,
        reference_expression: ReferenceExpression,
        authorized_mention: MentionId,
    },
}

impl fmt::Debug for ResolvedRequestView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LiteralCurrentTurn => formatter.write_str("LiteralCurrentTurn"),
            Self::WebReference {
                reference_expression,
                authorized_mention,
                ..
            } => formatter
                .debug_struct("ResolvedRequestView::WebReference")
                .field("original_request", &"<redacted>")
                .field("reference_expression", reference_expression)
                .field("authorized_mention", authorized_mention)
                .finish(),
        }
    }
}

pub(crate) enum ReferenceRoutingDecision {
    Proceed {
        view: ResolvedRequestView,
        permit: Option<ProviderQueryPermit>,
    },
    Blocked(ReferenceBlock),
    Confirmation(crate::reference_resolution::ConfirmationDisposition),
}

impl fmt::Debug for ReferenceRoutingDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proceed { view, permit } => formatter
                .debug_struct("ReferenceRoutingDecision::Proceed")
                .field("view", view)
                .field("has_permit", &permit.is_some())
                .finish(),
            Self::Blocked(block) => formatter
                .debug_tuple("ReferenceRoutingDecision::Blocked")
                .field(block)
                .finish(),
            Self::Confirmation(disposition) => formatter
                .debug_tuple("ReferenceRoutingDecision::Confirmation")
                .field(disposition)
                .finish(),
        }
    }
}

pub(crate) use super::query::{
    AuthorizationDenial, AuthorizationMethod, AuthorizedCandidateFetch, AuthorizedDirectFetch,
    AuthorizedSearch, DynamicCandidateSealer, Provider, ProviderAttemptIdentity, ProviderOperation,
    ProviderOperationKind, ProviderQueryAuthorization, ProviderQueryPermit, QueryLocale,
    QueryOperation, SealedDiscoveredCandidate, SealedQueryPlan,
};

#[derive(Debug)]
pub(crate) enum ReferenceResolution {
    ResolvedUserPublic {
        mention_id: MentionId,
        confidence: ResolutionConfidence,
        permit: ProviderQueryPermit,
    },
    ResolvedConfirmedPublic {
        mention_id: MentionId,
        confidence: ResolutionConfidence,
        permit: ProviderQueryPermit,
    },
    Ambiguous {
        clarification: ClarificationRequest,
    },
    MissingReferent {
        clarification: ClarificationRequest,
    },
    ConfirmationRequired {
        clarification: ClarificationRequest,
    },
    PrivateSourceDenied {
        reason: ResolutionReason,
        clarification: ClarificationRequest,
    },
    Expired {
        clarification: ClarificationRequest,
    },
    Unsupported {
        reason: ResolutionReason,
        clarification: ClarificationRequest,
    },
    RollbackLegacy,
}

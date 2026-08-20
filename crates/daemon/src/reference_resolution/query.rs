//! Closed query grammar and single-use provider capabilities.
//!
//! Plain query and URL bytes are kept here until an admission transaction has
//! committed. The public surface consists of opaque values and structural
//! enums. Content-bearing values have no public constructors, serializers, or
//! string accessors.

use super::{
    repository::{
        self, ReferenceRepository, RepositoryFault, ReserveAttemptV17Command,
        SealCandidateV17Command,
    },
    types::EntityKind,
};
use std::{fmt, sync::Arc};
use unicode_normalization::UnicodeNormalization;
use url::Url;

pub(crate) const CAPABILITY_VERSION: u32 = 1;
pub(crate) const SCHEMA_VERSION: u32 = 17;
pub(crate) const GRAMMAR_VERSION: u32 = 1;
pub(crate) const NORMALIZATION_VERSION: u32 = 1;
pub(crate) const PLAN_VERSION: u32 = 1;
pub(crate) const MAX_NAMED_TERM_BYTES: usize = 256;
pub(crate) const MAX_QUERY_BYTES: usize = 768;
pub(crate) const MAX_DIRECT_URL_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum QueryOperation {
    Research,
    Specifications,
    Verification,
    Lookup,
    Comparison,
    DirectFetch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum QueryKind {
    Person,
    Organization,
    Place,
    Product,
    TechnicalStandard,
    Document,
    PublicUrl,
    Unknown,
}

impl QueryKind {
    fn token(self) -> Option<&'static str> {
        Some(match self {
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Place => "place",
            Self::Product => "product",
            Self::TechnicalStandard => "standard",
            Self::Document => "document",
            Self::PublicUrl | Self::Unknown => return None,
        })
    }
}

impl From<EntityKind> for QueryKind {
    fn from(value: EntityKind) -> Self {
        match value {
            EntityKind::Person => Self::Person,
            EntityKind::Organization => Self::Organization,
            EntityKind::Place => Self::Place,
            EntityKind::Product => Self::Product,
            EntityKind::TechnicalStandard => Self::TechnicalStandard,
            EntityKind::DocumentTitle => Self::Document,
            EntityKind::PublicUrl => Self::PublicUrl,
            EntityKind::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum QueryFocus {
    GeneralPublicFacts,
    IdentityOfficeHolder,
    PopulationStatistic,
    Price,
    Weather,
    SoftwareVersion,
    TechnicalSpecification,
    LegalInformation,
    MedicalInformation,
    FinancialInformation,
}

impl QueryFocus {
    fn token(self) -> &'static str {
        match self {
            Self::GeneralPublicFacts => "public information",
            Self::IdentityOfficeHolder => "identity office holder",
            Self::PopulationStatistic => "population statistic",
            Self::Price => "price",
            Self::Weather => "weather",
            Self::SoftwareVersion => "software version",
            Self::TechnicalSpecification => "technical specifications",
            Self::LegalInformation => "legal information",
            Self::MedicalInformation => "medical information",
            Self::FinancialInformation => "financial information",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum QueryLocale {
    English,
    Slovak,
    Undetermined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Freshness {
    Stable,
    Current,
    Latest,
    Today,
}

impl Freshness {
    fn token(self) -> Option<&'static str> {
        match self {
            Self::Stable => None,
            Self::Current => Some("current"),
            Self::Latest => Some("latest"),
            Self::Today => Some("today"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AuthorityPreference {
    Any,
    OfficialFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Corroboration {
    OneAuthoritative,
    TwoIndependent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct QueryModifiers {
    pub(crate) locale: QueryLocale,
    pub(crate) freshness: Freshness,
    pub(crate) authority: AuthorityPreference,
    pub(crate) corroboration: Corroboration,
}

impl QueryModifiers {
    pub(crate) const fn stable_authoritative() -> Self {
        Self {
            locale: QueryLocale::Undetermined,
            freshness: Freshness::Stable,
            authority: AuthorityPreference::Any,
            corroboration: Corroboration::OneAuthoritative,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryError {
    Empty,
    InvalidTerm,
    Unsupported,
    InvalidUrl,
    TooLong,
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "empty query value",
            Self::InvalidTerm => "invalid named term",
            Self::Unsupported => "unsupported query combination",
            Self::InvalidUrl => "invalid public URL",
            Self::TooLong => "query value exceeds its byte limit",
        })
    }
}

fn is_forbidden_formatting(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}

fn normalize_named_term(value: &str) -> Result<String, QueryError> {
    let normalized = value.nfc().collect::<String>();
    if normalized.chars().any(is_forbidden_formatting) {
        return Err(QueryError::InvalidTerm);
    }
    let collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|character: char| {
        matches!(
            character,
            ' ' | '\t'
                | '\r'
                | '\n'
                | '.'
                | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '"'
                | '\''
                | '`'
        )
    });
    if trimmed.is_empty() {
        return Err(QueryError::Empty);
    }
    if trimmed.len() > MAX_NAMED_TERM_BYTES {
        return Err(QueryError::TooLong);
    }
    Ok(trimmed.to_owned())
}

fn quote_term(term: &str) -> String {
    let mut quoted = String::with_capacity(term.len() + 2);
    quoted.push('"');
    for character in term.chars() {
        if matches!(character, '\\' | '"') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

fn focus_compatible(kind: QueryKind, focus: QueryFocus) -> bool {
    match focus {
        QueryFocus::PopulationStatistic | QueryFocus::Weather => kind == QueryKind::Place,
        QueryFocus::Price | QueryFocus::SoftwareVersion => kind == QueryKind::Product,
        QueryFocus::TechnicalSpecification => {
            matches!(kind, QueryKind::Product | QueryKind::TechnicalStandard)
        }
        QueryFocus::IdentityOfficeHolder => {
            matches!(
                kind,
                QueryKind::Person | QueryKind::Organization | QueryKind::Place
            )
        }
        QueryFocus::MedicalInformation => {
            matches!(kind, QueryKind::Product | QueryKind::TechnicalStandard)
        }
        QueryFocus::GeneralPublicFacts
        | QueryFocus::LegalInformation
        | QueryFocus::FinancialInformation => kind.token().is_some(),
    }
}

fn append_suffixes(tokens: &mut Vec<String>, modifiers: QueryModifiers) {
    if let Some(freshness) = modifiers.freshness.token() {
        tokens.push(freshness.to_owned());
    }
    if modifiers.authority == AuthorityPreference::OfficialFirst {
        tokens.push("official".to_owned());
    }
}

fn compose_named_query(
    operation: QueryOperation,
    left: &str,
    left_kind: QueryKind,
    right: Option<(&str, QueryKind)>,
    focus: QueryFocus,
    modifiers: QueryModifiers,
) -> Result<String, QueryError> {
    let left = normalize_named_term(left)?;
    if left_kind == QueryKind::Unknown || left_kind == QueryKind::PublicUrl {
        return Err(QueryError::Unsupported);
    }
    if !focus_compatible(left_kind, focus) {
        return Err(QueryError::Unsupported);
    }
    let mut tokens = Vec::new();
    match operation {
        QueryOperation::Comparison => {
            let (right_term, right_kind) = right.ok_or(QueryError::Unsupported)?;
            let right_term = normalize_named_term(right_term)?;
            if right_kind != left_kind
                || right_kind == QueryKind::Unknown
                || right_kind == QueryKind::PublicUrl
                || right_term == left
            {
                return Err(QueryError::Unsupported);
            }
            if !focus_compatible(right_kind, focus) {
                return Err(QueryError::Unsupported);
            }
            tokens.push(quote_term(&left));
            tokens.push("versus".to_owned());
            tokens.push(quote_term(&right_term));
            tokens.push(left_kind.token().ok_or(QueryError::Unsupported)?.to_owned());
            tokens.push(focus.token().to_owned());
            tokens.push("comparison".to_owned());
        }
        QueryOperation::Research => {
            tokens.push(quote_term(&left));
            tokens.push(left_kind.token().ok_or(QueryError::Unsupported)?.to_owned());
            tokens.push(focus.token().to_owned());
            tokens.push("research".to_owned());
        }
        QueryOperation::Specifications => {
            if !matches!(left_kind, QueryKind::Product | QueryKind::TechnicalStandard) {
                return Err(QueryError::Unsupported);
            }
            tokens.push(quote_term(&left));
            tokens.push(left_kind.token().ok_or(QueryError::Unsupported)?.to_owned());
            tokens.push("technical specifications".to_owned());
        }
        QueryOperation::Verification => {
            tokens.push(quote_term(&left));
            tokens.push(left_kind.token().ok_or(QueryError::Unsupported)?.to_owned());
            tokens.push(focus.token().to_owned());
            tokens.push("verification".to_owned());
        }
        QueryOperation::Lookup => {
            tokens.push(quote_term(&left));
            tokens.push(left_kind.token().ok_or(QueryError::Unsupported)?.to_owned());
            tokens.push(focus.token().to_owned());
        }
        QueryOperation::DirectFetch => return Err(QueryError::Unsupported),
    }
    append_suffixes(&mut tokens, modifiers);
    let query = tokens.join(" ");
    if query.len() > MAX_QUERY_BYTES {
        return Err(QueryError::TooLong);
    }
    Ok(query)
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    (0..bytes.len()).all(|index| {
        bytes[index] != b'%'
            || (index + 2 < bytes.len()
                && bytes[index + 1].is_ascii_hexdigit()
                && bytes[index + 2].is_ascii_hexdigit())
    })
}

fn is_unsafe_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return true;
    }
    let Ok(address) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    match address {
        std::net::IpAddr::V4(address) => {
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_unspecified()
                || address.octets()[0] == 192
                    && address.octets()[1] == 0
                    && address.octets()[2] == 2
                || address.octets()[0] == 198
                    && address.octets()[1] == 51
                    && address.octets()[2] == 100
                || address.octets()[0] == 203
                    && address.octets()[1] == 0
                    && address.octets()[2] == 113
        }
        std::net::IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
        }
    }
}

fn normalize_public_url(value: &str) -> Result<String, QueryError> {
    if value.chars().any(is_forbidden_formatting) {
        return Err(QueryError::InvalidUrl);
    }
    let mut url = Url::parse(value).map_err(|_| QueryError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(QueryError::InvalidUrl);
    }
    let host = url.host_str().ok_or(QueryError::InvalidUrl)?;
    if is_unsafe_host(host) {
        return Err(QueryError::InvalidUrl);
    }
    if let Some(port) = url.port() {
        let is_default =
            (url.scheme() == "http" && port == 80) || (url.scheme() == "https" && port == 443);
        if is_default {
            url.set_port(None).map_err(|_| QueryError::InvalidUrl)?;
        }
    }
    url.set_fragment(None);
    let query = url.query().unwrap_or_default();
    if !valid_percent_encoding(query) {
        return Err(QueryError::InvalidUrl);
    }
    let mut pairs = Vec::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let lower_key = key.to_ascii_lowercase();
        if lower_key.starts_with("utm_")
            || matches!(
                lower_key.as_str(),
                "fbclid"
                    | "gclid"
                    | "token"
                    | "access-token"
                    | "api-key"
                    | "secret"
                    | "password"
                    | "auth"
                    | "authorization"
                    | "signature"
            )
        {
            if matches!(
                lower_key.as_str(),
                "fbclid"
                    | "gclid"
                    | "token"
                    | "access-token"
                    | "api-key"
                    | "secret"
                    | "password"
                    | "auth"
                    | "authorization"
                    | "signature"
            ) {
                return Err(QueryError::InvalidUrl);
            }
            continue;
        }
        pairs.push((key, value));
    }
    pairs.sort_unstable();
    let rebuilt = pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    if rebuilt.is_empty() {
        url.set_query(None);
    } else {
        url.set_query(Some(&rebuilt));
    }
    let normalized = url.to_string();
    if normalized.len() > MAX_DIRECT_URL_BYTES {
        return Err(QueryError::TooLong);
    }
    Ok(normalized)
}

pub(crate) fn normalize_public_url_for_adapter(value: &str) -> Result<String, QueryError> {
    normalize_public_url(value)
}

#[cfg(test)]
pub(crate) struct QueryReferentInput {
    term: String,
    kind: QueryKind,
}

#[cfg(test)]
impl QueryReferentInput {
    pub(crate) fn named(term: impl Into<String>, kind: QueryKind) -> Self {
        Self {
            term: term.into(),
            kind,
        }
    }
}

#[cfg(test)]
pub(crate) fn compose_query_for_test(
    operation: QueryOperation,
    referent: QueryReferentInput,
    focus: QueryFocus,
    modifiers: QueryModifiers,
) -> Result<String, QueryError> {
    compose_named_query(
        operation,
        &referent.term,
        referent.kind,
        None,
        focus,
        modifiers,
    )
}

#[cfg(test)]
pub(crate) fn compose_comparison_query_for_test(
    left: QueryReferentInput,
    right: QueryReferentInput,
    focus: QueryFocus,
    modifiers: QueryModifiers,
) -> Result<String, QueryError> {
    compose_named_query(
        QueryOperation::Comparison,
        &left.term,
        left.kind,
        Some((&right.term, right.kind)),
        focus,
        modifiers,
    )
}

#[cfg(test)]
pub(crate) fn normalize_public_term_for_test(value: &str) -> String {
    normalize_named_term(value).expect("valid synthetic term")
}

#[cfg(test)]
pub(crate) fn test_authorized_direct_fetch(url: &str) -> AuthorizedDirectFetch {
    AuthorizedDirectFetch {
        url: normalize_public_url(url).expect("synthetic public URL"),
        attempt: ProviderAttemptIdentity {
            reservation_id: "synthetic-reservation".to_owned(),
            attempt_number: 1,
            operation_hmac: [0; 32],
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorizationMethod {
    CurrentUser,
    CanonicalWeb,
    Confirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provider {
    Tavily,
    DuckDuckGo,
    Wikipedia,
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderOperationKind {
    Search,
    CandidateFetch,
    DirectFetch,
}

pub(crate) struct SealedQueryPlan {
    operation: QueryOperation,
    focus: QueryFocus,
    modifiers: QueryModifiers,
    locale: QueryLocale,
    authorization_id: String,
    session_id: String,
    initiating_turn_id: String,
    execution_turn_id: String,
    referent_set_hmac: [u8; 32],
    sealed_plan_hmac: [u8; 32],
    search_budget: u8,
    fetch_budget: u8,
    providers: Vec<Provider>,
    query: Option<String>,
    normalized_url: Option<String>,
    authorization_method: AuthorizationMethod,
    provider_scope: &'static str,
    compatibility_epoch: u32,
}

impl SealedQueryPlan {
    fn schema_version(&self) -> u32 {
        SCHEMA_VERSION
    }
    fn grammar_version(&self) -> u32 {
        GRAMMAR_VERSION
    }
    fn normalization_version(&self) -> u32 {
        NORMALIZATION_VERSION
    }
    fn compatibility_epoch(&self) -> u32 {
        self.compatibility_epoch
    }
}

impl fmt::Debug for SealedQueryPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedQueryPlan")
            .field("operation", &self.operation)
            .field("focus", &self.focus)
            .field("search_budget", &self.search_budget)
            .field("fetch_budget", &self.fetch_budget)
            .field("has_query", &self.query.is_some())
            .field("has_url", &self.normalized_url.is_some())
            .finish()
    }
}

pub(crate) struct AuthorizedReferent {
    mention_id: String,
    referent_id: String,
    term: String,
    kind: QueryKind,
    authorization_method: AuthorizationMethod,
    language: QueryLocale,
    comparison_side: Option<u8>,
}

pub(crate) enum AuthorizedReferentSet {
    Single(AuthorizedReferent),
    Comparison {
        left: AuthorizedReferent,
        right: AuthorizedReferent,
    },
}

impl fmt::Debug for AuthorizedReferent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedReferent")
            .field("has_mention", &true)
            .field("kind", &self.kind)
            .field("authorization_method", &self.authorization_method)
            .field("comparison_side", &self.comparison_side)
            .finish()
    }
}

impl fmt::Debug for AuthorizedReferentSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Single(_) => formatter.write_str("AuthorizedReferentSet::Single(<sealed>)"),
            Self::Comparison { .. } => {
                formatter.write_str("AuthorizedReferentSet::Comparison(<sealed>)")
            }
        }
    }
}

pub(crate) struct SealPlanRequest {
    operation: QueryOperation,
    focus: QueryFocus,
    modifiers: QueryModifiers,
    referents: AuthorizedReferentSet,
    authorization_id: String,
    session_id: String,
    initiating_turn_id: String,
    execution_turn_id: String,
    authorization_method: AuthorizationMethod,
    provider_scope: &'static str,
    process_epoch: u32,
    configuration_epoch: u32,
    resolver_mode_epoch: u32,
    issued_at_ms: i64,
}

fn provider_token(provider: Provider) -> &'static str {
    match provider {
        Provider::Tavily => "tavily",
        Provider::DuckDuckGo => "duckduckgo",
        Provider::Wikipedia => "wikipedia",
        Provider::Direct => "direct",
    }
}

fn method_token(method: AuthorizationMethod) -> &'static str {
    match method {
        AuthorizationMethod::CurrentUser => "current_user",
        AuthorizationMethod::CanonicalWeb => "canonical_web",
        AuthorizationMethod::Confirmed => "confirmed",
    }
}

fn encode_part(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u32).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn referent_parts(set: &AuthorizedReferentSet) -> Vec<Vec<u8>> {
    let values = match set {
        AuthorizedReferentSet::Single(referent) => vec![referent],
        AuthorizedReferentSet::Comparison { left, right } => vec![left, right],
    };
    values
        .into_iter()
        .map(|referent| {
            let mut encoded = Vec::new();
            encode_part(&mut encoded, referent.mention_id.as_bytes());
            encode_part(&mut encoded, referent.referent_id.as_bytes());
            encode_part(
                &mut encoded,
                referent.kind.token().unwrap_or("unknown").as_bytes(),
            );
            encode_part(
                &mut encoded,
                method_token(referent.authorization_method).as_bytes(),
            );
            encode_part(&mut encoded, referent.term.as_bytes());
            encode_part(&mut encoded, format!("{:?}", referent.language).as_bytes());
            encode_part(
                &mut encoded,
                &referent.comparison_side.unwrap_or(255).to_be_bytes(),
            );
            encoded
        })
        .collect()
}

fn providers_for(operation: QueryOperation, corroboration: Corroboration) -> Vec<Provider> {
    if operation == QueryOperation::DirectFetch {
        return vec![Provider::Direct];
    }
    match corroboration {
        Corroboration::OneAuthoritative => vec![Provider::Wikipedia],
        Corroboration::TwoIndependent => vec![Provider::Wikipedia, Provider::DuckDuckGo],
    }
}

fn budgets_for(operation: QueryOperation, modifiers: QueryModifiers) -> (u8, u8) {
    match operation {
        QueryOperation::DirectFetch => (0, 1),
        QueryOperation::Research | QueryOperation::Verification | QueryOperation::Comparison => {
            (2, 5)
        }
        QueryOperation::Specifications => (2, 3),
        QueryOperation::Lookup => match (modifiers.freshness, modifiers.corroboration) {
            (Freshness::Stable, Corroboration::OneAuthoritative) => (1, 3),
            _ => (2, 5),
        },
    }
}

fn plan_bytes(plan: &SealPlanRequest, query: Option<&str>, url: Option<&str>) -> Vec<u8> {
    let mut encoded = Vec::new();
    encode_part(&mut encoded, format!("{:?}", plan.operation).as_bytes());
    encode_part(&mut encoded, plan.focus.token().as_bytes());
    encode_part(&mut encoded, format!("{:?}", plan.modifiers).as_bytes());
    encode_part(&mut encoded, plan.provider_scope.as_bytes());
    encode_part(&mut encoded, plan.authorization_id.as_bytes());
    encode_part(&mut encoded, plan.session_id.as_bytes());
    encode_part(&mut encoded, plan.initiating_turn_id.as_bytes());
    encode_part(&mut encoded, plan.execution_turn_id.as_bytes());
    encode_part(
        &mut encoded,
        method_token(plan.authorization_method).as_bytes(),
    );
    encode_part(&mut encoded, &plan.process_epoch.to_be_bytes());
    encode_part(&mut encoded, &plan.configuration_epoch.to_be_bytes());
    encode_part(&mut encoded, &plan.resolver_mode_epoch.to_be_bytes());
    for referent in referent_parts(&plan.referents) {
        encode_part(&mut encoded, &referent);
    }
    encode_part(&mut encoded, query.unwrap_or_default().as_bytes());
    encode_part(&mut encoded, url.unwrap_or_default().as_bytes());
    encoded
}

pub(crate) fn seal_query_plan(
    repository: Arc<repository::SqliteRepository>,
    request: SealPlanRequest,
) -> Result<(ProviderQueryPermit, Vec<ProviderOperation>), QueryError> {
    validate_referents(&request.referents)?;
    match (&request.referents, request.operation) {
        (AuthorizedReferentSet::Comparison { .. }, QueryOperation::Comparison)
        | (AuthorizedReferentSet::Single(_), QueryOperation::Research)
        | (AuthorizedReferentSet::Single(_), QueryOperation::Specifications)
        | (AuthorizedReferentSet::Single(_), QueryOperation::Verification)
        | (AuthorizedReferentSet::Single(_), QueryOperation::Lookup)
        | (AuthorizedReferentSet::Single(_), QueryOperation::DirectFetch) => {}
        _ => return Err(QueryError::Unsupported),
    }
    let (query, normalized_url) = match &request.referents {
        AuthorizedReferentSet::Single(referent)
            if request.operation == QueryOperation::DirectFetch =>
        {
            if referent.kind != QueryKind::PublicUrl {
                return Err(QueryError::Unsupported);
            }
            (None, Some(normalize_public_url(&referent.term)?))
        }
        AuthorizedReferentSet::Single(referent) => (
            Some(compose_named_query(
                request.operation,
                &referent.term,
                referent.kind,
                None,
                request.focus,
                request.modifiers,
            )?),
            None,
        ),
        AuthorizedReferentSet::Comparison { left, right } => (
            Some(compose_named_query(
                QueryOperation::Comparison,
                &left.term,
                left.kind,
                Some((&right.term, right.kind)),
                request.focus,
                request.modifiers,
            )?),
            None,
        ),
    };
    let referent_bytes = referent_parts(&request.referents)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let referent_set_hmac = repository
        .structural_hmac(
            &request.authorization_id,
            &request.session_id,
            "referent_set_v17",
            &request.initiating_turn_id,
            &referent_bytes,
        )
        .map_err(|_| QueryError::Unsupported)?;
    let plan_bytes = plan_bytes(&request, query.as_deref(), normalized_url.as_deref());
    let sealed_plan_hmac = repository
        .structural_hmac(
            &request.authorization_id,
            &request.session_id,
            "sealed_plan_v17",
            &request.initiating_turn_id,
            &plan_bytes,
        )
        .map_err(|_| QueryError::Unsupported)?;
    let permit_nonce_hmac = repository
        .structural_hmac(
            &request.authorization_id,
            &request.session_id,
            "permit_nonce_v17",
            &request.execution_turn_id,
            uuid::Uuid::new_v4().as_bytes(),
        )
        .map_err(|_| QueryError::Unsupported)?;
    let (search_budget, fetch_budget) = budgets_for(request.operation, request.modifiers);
    let providers = providers_for(request.operation, request.modifiers.corroboration);
    let expires_at_ms = request
        .issued_at_ms
        .checked_add(300_000)
        .ok_or(QueryError::Unsupported)?;
    let authorization_hmac = repository
        .structural_hmac(
            &request.authorization_id,
            &request.session_id,
            "authorization",
            &request.execution_turn_id,
            request.authorization_id.as_bytes(),
        )
        .map_err(|_| QueryError::Unsupported)?;
    let plan = SealedQueryPlan {
        operation: request.operation,
        focus: request.focus,
        modifiers: request.modifiers,
        locale: request.modifiers.locale,
        authorization_id: request.authorization_id,
        session_id: request.session_id,
        initiating_turn_id: request.initiating_turn_id,
        execution_turn_id: request.execution_turn_id,
        referent_set_hmac,
        sealed_plan_hmac,
        search_budget,
        fetch_budget,
        providers: providers.clone(),
        query,
        normalized_url,
        authorization_method: request.authorization_method,
        provider_scope: request.provider_scope,
        compatibility_epoch: 1,
    };
    let permit = ProviderQueryPermit {
        plan,
        process_epoch: request.process_epoch,
        configuration_epoch: request.configuration_epoch,
        resolver_mode_epoch: request.resolver_mode_epoch,
        permit_nonce_hmac,
        issued_at_ms: request.issued_at_ms,
        expires_at_ms,
        repository,
        authorization_hmac,
    };
    let mut operations = Vec::new();
    for (slot, provider) in providers.into_iter().enumerate() {
        let kind = if request.operation == QueryOperation::DirectFetch {
            ProviderOperationKind::DirectFetch
        } else {
            ProviderOperationKind::Search
        };
        let mut input = Vec::new();
        input.extend_from_slice(&(slot as u16).to_be_bytes());
        input.extend_from_slice(format!("{:?}", kind).as_bytes());
        input.extend_from_slice(provider_token(provider).as_bytes());
        input.extend_from_slice(&sealed_plan_hmac);
        let operation_hmac = permit
            .repository
            .structural_hmac(
                &permit.plan.authorization_id,
                &permit.plan.session_id,
                "operation_v17",
                &permit.plan.execution_turn_id,
                &input,
            )
            .map_err(|_| QueryError::Unsupported)?;
        operations.push(ProviderOperation {
            slot: slot as u16,
            kind,
            provider,
            operation_hmac,
            variant_id: None,
            variant_hmac: None,
            attempt_number: 1,
            parent_reservation_id: None,
            parent_reservation_hmac: None,
            candidate: None,
            retry_operation_hmac: None,
        });
    }
    let _ = authorization_hmac;
    Ok((permit, operations))
}

fn validate_referents(set: &AuthorizedReferentSet) -> Result<(), QueryError> {
    let referents = match set {
        AuthorizedReferentSet::Single(referent) => vec![referent],
        AuthorizedReferentSet::Comparison { left, right } => {
            if left.referent_id == right.referent_id
                || left.kind == QueryKind::Unknown
                || right.kind == QueryKind::Unknown
                || left.kind != right.kind
            {
                return Err(QueryError::Unsupported);
            }
            vec![left, right]
        }
    };
    if referents.iter().any(|referent| {
        referent.kind == QueryKind::Unknown
            || referent.mention_id.is_empty()
            || referent.referent_id.is_empty()
            || referent.term.is_empty()
    }) {
        return Err(QueryError::Unsupported);
    }
    Ok(())
}

pub(crate) struct ProviderQueryPermit {
    plan: SealedQueryPlan,
    process_epoch: u32,
    configuration_epoch: u32,
    resolver_mode_epoch: u32,
    permit_nonce_hmac: [u8; 32],
    issued_at_ms: i64,
    expires_at_ms: i64,
    repository: Arc<repository::SqliteRepository>,
    authorization_hmac: [u8; 32],
}

impl fmt::Debug for ProviderQueryPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderQueryPermit")
            .field("authorization_id", &"<opaque>")
            .field("process_epoch", &self.process_epoch)
            .field("configuration_epoch", &self.configuration_epoch)
            .field("resolver_mode_epoch", &self.resolver_mode_epoch)
            .field("has_nonce", &true)
            .finish()
    }
}

pub(crate) struct ProviderOperation {
    slot: u16,
    kind: ProviderOperationKind,
    provider: Provider,
    operation_hmac: [u8; 32],
    variant_id: Option<String>,
    variant_hmac: Option<[u8; 32]>,
    attempt_number: u8,
    parent_reservation_id: Option<String>,
    parent_reservation_hmac: Option<[u8; 32]>,
    candidate: Option<SealedDiscoveredCandidate>,
    retry_operation_hmac: Option<[u8; 32]>,
}

impl ProviderOperation {
    fn attempt_number(&self) -> u8 {
        self.attempt_number
    }
    fn variant_hmac(&self) -> Option<[u8; 32]> {
        self.variant_hmac
    }
    fn parent_reservation_id(&self) -> Option<String> {
        self.parent_reservation_id.clone().or_else(|| {
            self.candidate
                .as_ref()
                .map(|candidate| candidate.parent_reservation_id.clone())
        })
    }
    fn parent_reservation_hmac(&self) -> Option<[u8; 32]> {
        self.parent_reservation_hmac.or_else(|| {
            self.candidate
                .as_ref()
                .map(|candidate| candidate.parent_reservation_hmac)
        })
    }
    fn candidate_binding_id(&self) -> Option<String> {
        self.candidate
            .as_ref()
            .map(|candidate| candidate.candidate_binding_id.clone())
    }
    fn candidate_binding_hmac(&self) -> Option<[u8; 32]> {
        self.candidate
            .as_ref()
            .map(|candidate| candidate.binding_hmac)
    }
}

impl fmt::Debug for ProviderOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderOperation")
            .field("slot", &self.slot)
            .field("kind", &self.kind)
            .field("provider", &self.provider)
            .field("has_variant", &self.variant_id.is_some())
            .field("has_candidate", &self.candidate.is_some())
            .finish()
    }
}

pub(crate) struct AuthorizedSearch {
    provider: Provider,
    query: String,
    locale: QueryLocale,
    attempt: ProviderAttemptIdentity,
    plan_hmac: [u8; 32],
    authorization_id: String,
    session_id: String,
    execution_turn_id: String,
    authorization_hmac: [u8; 32],
    permit_nonce_hmac: [u8; 32],
    repository: Arc<repository::SqliteRepository>,
}

pub(crate) struct AuthorizedCandidateFetch {
    candidate: SealedDiscoveredCandidate,
    attempt: ProviderAttemptIdentity,
}

pub(crate) struct AuthorizedDirectFetch {
    url: String,
    attempt: ProviderAttemptIdentity,
}

pub(crate) enum ProviderQueryAuthorization {
    AuthorizedSearch(AuthorizedSearch),
    AuthorizedCandidateFetch(AuthorizedCandidateFetch),
    AuthorizedDirectFetch(AuthorizedDirectFetch),
    Denied { reason: AuthorizationDenial },
}

impl fmt::Debug for ProviderQueryAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthorizedSearch(_) => formatter.write_str("AuthorizedSearch(<sealed>)"),
            Self::AuthorizedCandidateFetch(_) => {
                formatter.write_str("AuthorizedCandidateFetch(<sealed>)")
            }
            Self::AuthorizedDirectFetch(_) => {
                formatter.write_str("AuthorizedDirectFetch(<sealed>)")
            }
            Self::Denied { reason } => formatter
                .debug_struct("Denied")
                .field("reason", reason)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorizationDenial {
    Disabled,
    ResolverUnavailable,
    InvalidVersion,
    Expired,
    Replay,
    MismatchedBinding,
    InvalidOperation,
    BudgetExhausted,
    UnsafeValue,
    RepositoryFault,
    ConfigurationMismatch,
    AmbiguousCommit,
}

#[derive(PartialEq, Eq)]
pub(crate) struct ProviderAttemptIdentity {
    reservation_id: String,
    attempt_number: u8,
    operation_hmac: [u8; 32],
}

impl ProviderAttemptIdentity {
    fn duplicate_for_internal_use(&self) -> Self {
        Self {
            reservation_id: self.reservation_id.clone(),
            attempt_number: self.attempt_number,
            operation_hmac: self.operation_hmac,
        }
    }
}

impl fmt::Debug for ProviderAttemptIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderAttemptIdentity")
            .field("attempt_number", &self.attempt_number)
            .field("has_reservation", &true)
            .finish()
    }
}

pub(crate) struct SealedDiscoveredCandidate {
    candidate_binding_id: String,
    authorization_id: String,
    parent_reservation_id: String,
    parent_reservation_hmac: [u8; 32],
    provider: Provider,
    ordinal: u32,
    normalized_url: String,
    source_identity_hmac: [u8; 32],
    binding_hmac: [u8; 32],
    retry_relationship_hmac: [u8; 32],
}

pub(crate) struct DynamicCandidateSealer {
    provider: Provider,
    authorization_id: String,
    session_id: String,
    execution_turn_id: String,
    authorization_hmac: [u8; 32],
    permit_nonce_hmac: [u8; 32],
    plan_hmac: [u8; 32],
    parent: ProviderAttemptIdentity,
    repository: Arc<repository::SqliteRepository>,
}

impl fmt::Debug for AuthorizedSearch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedSearch")
            .field("provider", &self.provider)
            .field("locale", &self.locale)
            .field("has_query", &true)
            .field("has_attempt", &true)
            .finish()
    }
}

impl fmt::Debug for AuthorizedCandidateFetch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedCandidateFetch")
            .field("provider", &self.candidate.provider)
            .field("has_url", &true)
            .field("has_attempt", &true)
            .finish()
    }
}

impl fmt::Debug for AuthorizedDirectFetch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedDirectFetch")
            .field("has_url", &true)
            .field("has_attempt", &true)
            .finish()
    }
}

impl fmt::Debug for DynamicCandidateSealer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DynamicCandidateSealer")
            .field("provider", &self.provider)
            .field("has_parent", &true)
            .finish()
    }
}

impl fmt::Debug for SealedDiscoveredCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedDiscoveredCandidate")
            .field("provider", &self.provider)
            .field("ordinal", &self.ordinal)
            .field("has_parent", &true)
            .finish()
    }
}

impl AuthorizedSearch {
    pub(crate) fn provider(&self) -> Provider {
        self.provider
    }
    pub(crate) fn locale(&self) -> QueryLocale {
        self.locale
    }
    pub(crate) fn attempt(&self) -> &ProviderAttemptIdentity {
        &self.attempt
    }
    pub(crate) fn with_query<R>(&self, use_query: impl FnOnce(&str) -> R) -> R {
        use_query(&self.query)
    }
    pub(crate) fn candidate_sealer(&self) -> DynamicCandidateSealer {
        DynamicCandidateSealer {
            provider: self.provider,
            authorization_id: self.authorization_id.clone(),
            session_id: self.session_id.clone(),
            execution_turn_id: self.execution_turn_id.clone(),
            authorization_hmac: self.authorization_hmac,
            permit_nonce_hmac: self.permit_nonce_hmac,
            plan_hmac: self.plan_hmac,
            parent: self.attempt.duplicate_for_internal_use(),
            repository: Arc::clone(&self.repository),
        }
    }
}

impl DynamicCandidateSealer {
    pub(crate) async fn seal_candidate(
        &self,
        result_ordinal: u32,
        url: &str,
        source_identity: &str,
        fetch_slot: u8,
    ) -> Result<ProviderOperation, AuthorizationDenial> {
        if fetch_slot >= 5 {
            return Err(AuthorizationDenial::BudgetExhausted);
        }
        let normalized_url =
            normalize_public_url(url).map_err(|_| AuthorizationDenial::UnsafeValue)?;
        let url_hmac = self.hmac("candidate_url", normalized_url.as_bytes())?;
        let source_hmac = self.hmac("candidate_source", source_identity.as_bytes())?;
        let capability_input = [&self.permit_nonce_hmac[..], normalized_url.as_bytes()].concat();
        let candidate_capability_hmac = self.hmac("candidate_capability", &capability_input)?;
        let retry_input = [self.parent.reservation_id.as_bytes(), &url_hmac].concat();
        let retry_relationship_hmac = self.hmac("candidate_retry", &retry_input)?;
        let binding_input = [
            self.authorization_id.as_bytes(),
            self.parent.reservation_id.as_bytes(),
            &result_ordinal.to_be_bytes(),
            &url_hmac,
            &source_hmac,
            &candidate_capability_hmac,
            &retry_relationship_hmac,
        ]
        .concat();
        let binding_hmac = self.hmac("candidate_binding", &binding_input)?;
        let now = system_now_ms();
        let expires_at_ms = now
            .checked_add(300_000)
            .ok_or(AuthorizationDenial::RepositoryFault)?;
        let readback = self
            .repository
            .seal_dynamic_candidate_v17(SealCandidateV17Command {
                authorization_id: self.authorization_id.clone(),
                authorization_hmac: self.authorization_hmac,
                fetch_slot: i64::from(fetch_slot),
                parent_reservation_id: self.parent.reservation_id.clone(),
                parent_reservation_hmac: self.parent_operation_hmac()?,
                discovery_provider_hmac: self
                    .hmac("provider", provider_token(self.provider).as_bytes())?,
                normalized_url_hmac: url_hmac,
                source_identity_hmac: source_hmac,
                candidate_capability_hmac,
                retry_relationship_hmac,
                result_ordinal: i64::from(result_ordinal),
                binding_hmac,
                created_at_ms: now,
                expires_at_ms,
                schema_version: i64::from(SCHEMA_VERSION),
                hmac_key_version: 1,
            })
            .await
            .map_err(repository_denial)?;
        let candidate = SealedDiscoveredCandidate {
            candidate_binding_id: readback.candidate_binding_id,
            authorization_id: self.authorization_id.clone(),
            parent_reservation_id: self.parent.reservation_id.clone(),
            parent_reservation_hmac: self.parent_operation_hmac()?,
            provider: self.provider,
            ordinal: result_ordinal,
            normalized_url,
            source_identity_hmac: source_hmac,
            binding_hmac: readback.binding_hmac,
            retry_relationship_hmac,
        };
        let operation_slot = 2 + u16::from(fetch_slot);
        let operation_hmac = self.operation_hmac(operation_slot, 1, candidate.binding_hmac)?;
        let retry_operation_hmac =
            self.operation_hmac(operation_slot, 2, candidate.binding_hmac)?;
        let operation = ProviderOperation {
            slot: operation_slot,
            kind: ProviderOperationKind::CandidateFetch,
            provider: self.provider,
            operation_hmac,
            variant_id: None,
            variant_hmac: None,
            attempt_number: 1,
            parent_reservation_id: Some(candidate.parent_reservation_id.clone()),
            parent_reservation_hmac: Some(candidate.parent_reservation_hmac),
            candidate: Some(candidate),
            retry_operation_hmac: Some(retry_operation_hmac),
        };
        Ok(operation)
    }

    /// Consume a pre-sealed candidate operation to select its one retry slot.
    /// The retry remains bound to the same candidate and parent reservation.
    pub(crate) fn retry_candidate(
        mut operation: ProviderOperation,
    ) -> Result<ProviderOperation, AuthorizationDenial> {
        if operation.kind != ProviderOperationKind::CandidateFetch
            || operation.attempt_number != 1
            || operation.candidate.is_none()
        {
            return Err(AuthorizationDenial::InvalidOperation);
        }
        operation.attempt_number = 2;
        operation.operation_hmac = operation
            .retry_operation_hmac
            .take()
            .ok_or(AuthorizationDenial::InvalidOperation)?;
        Ok(operation)
    }

    fn hmac(&self, field: &'static str, value: &[u8]) -> Result<[u8; 32], AuthorizationDenial> {
        self.repository
            .structural_hmac(
                &self.authorization_id,
                &self.session_id,
                field,
                &self.execution_turn_id,
                value,
            )
            .map_err(|_| AuthorizationDenial::RepositoryFault)
    }

    fn parent_operation_hmac(&self) -> Result<[u8; 32], AuthorizationDenial> {
        Ok(self.parent.operation_hmac)
    }

    fn operation_hmac(
        &self,
        slot: u16,
        attempt: u8,
        binding_hmac: [u8; 32],
    ) -> Result<[u8; 32], AuthorizationDenial> {
        let input = [
            &slot.to_be_bytes()[..],
            &[attempt],
            provider_token(self.provider).as_bytes(),
            &self.plan_hmac,
            &binding_hmac,
        ]
        .concat();
        self.hmac("operation_v17", &input)
    }
}

impl AuthorizedDirectFetch {
    pub(crate) fn attempt(&self) -> &ProviderAttemptIdentity {
        &self.attempt
    }
    pub(crate) fn with_url<R>(&self, use_url: impl FnOnce(&str) -> R) -> R {
        use_url(&self.url)
    }
}

impl AuthorizedCandidateFetch {
    pub(crate) fn attempt(&self) -> &ProviderAttemptIdentity {
        &self.attempt
    }
    pub(crate) fn provider(&self) -> Provider {
        self.candidate.provider
    }
    pub(crate) fn with_url<R>(&self, use_url: impl FnOnce(&str) -> R) -> R {
        use_url(&self.candidate.normalized_url)
    }
}

pub(crate) async fn admit_provider_query(
    permit: &ProviderQueryPermit,
    operation: ProviderOperation,
) -> ProviderQueryAuthorization {
    if !cfg!(test) {
        return ProviderQueryAuthorization::Denied {
            reason: AuthorizationDenial::Disabled,
        };
    }
    admit_provider_query_with_context(
        permit,
        operation,
        AdmissionContext {
            enabled: true,
            configuration_epoch: permit.configuration_epoch,
            resolver_mode_epoch: permit.resolver_mode_epoch,
            process_epoch: permit.process_epoch,
        },
    )
    .await
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AdmissionContext {
    pub(crate) enabled: bool,
    pub(crate) configuration_epoch: u32,
    pub(crate) resolver_mode_epoch: u32,
    pub(crate) process_epoch: u32,
}

async fn admit_provider_query_with_context(
    permit: &ProviderQueryPermit,
    operation: ProviderOperation,
    context: AdmissionContext,
) -> ProviderQueryAuthorization {
    if !context.enabled {
        return ProviderQueryAuthorization::Denied {
            reason: AuthorizationDenial::Disabled,
        };
    }
    if permit.plan.provider_scope == "" || permit.plan.authorization_id.is_empty() {
        return ProviderQueryAuthorization::Denied {
            reason: AuthorizationDenial::InvalidVersion,
        };
    }
    if context.configuration_epoch != permit.configuration_epoch
        || context.resolver_mode_epoch != permit.resolver_mode_epoch
        || context.process_epoch != permit.process_epoch
        || permit.plan.schema_version() != SCHEMA_VERSION
        || permit.plan.grammar_version() != GRAMMAR_VERSION
        || permit.plan.normalization_version() != NORMALIZATION_VERSION
    {
        return ProviderQueryAuthorization::Denied {
            reason: AuthorizationDenial::ConfigurationMismatch,
        };
    }
    let now = system_now_ms();
    if now >= permit.expires_at_ms || now < permit.issued_at_ms {
        return ProviderQueryAuthorization::Denied {
            reason: AuthorizationDenial::Expired,
        };
    }

    let is_search = matches!(operation.kind, ProviderOperationKind::Search);
    let operation_provider_hmac = match permit.repository.structural_hmac(
        &permit.plan.authorization_id,
        &permit.plan.session_id,
        "provider",
        &permit.plan.execution_turn_id,
        provider_token(operation.provider).as_bytes(),
    ) {
        Ok(value) => value,
        Err(_) => {
            return ProviderQueryAuthorization::Denied {
                reason: AuthorizationDenial::RepositoryFault,
            }
        }
    };
    let command = ReserveAttemptV17Command {
        authorization_id: permit.plan.authorization_id.clone(),
        authorization_hmac: permit.authorization_hmac,
        session_id: permit.plan.session_id.clone(),
        initiating_turn_id: permit.plan.initiating_turn_id.clone(),
        execution_turn_id: permit.plan.execution_turn_id.clone(),
        authorization_method: method_token(permit.plan.authorization_method),
        provider_scope: permit.plan.provider_scope,
        is_search,
        operation_slot: i64::from(operation.slot),
        variant_id: operation.variant_id.clone(),
        variant_hmac: operation.variant_hmac(),
        attempt_number: i64::from(operation.attempt_number()),
        parent_reservation_id: operation.parent_reservation_id(),
        parent_reservation_hmac: operation.parent_reservation_hmac(),
        candidate_binding_id: operation.candidate_binding_id(),
        candidate_binding_hmac: operation.candidate_binding_hmac(),
        provider_hmac: operation_provider_hmac,
        operation_hmac: operation.operation_hmac,
        sealed_plan_hmac: permit.plan.sealed_plan_hmac,
        permit_nonce_hmac: permit.permit_nonce_hmac,
        plan_version: i64::from(PLAN_VERSION),
        schema_version: i64::from(SCHEMA_VERSION),
        grammar_version: i64::from(GRAMMAR_VERSION),
        normalization_version: i64::from(NORMALIZATION_VERSION),
        compatibility_epoch: i64::from(permit.plan.compatibility_epoch()),
        configuration_epoch: i64::from(permit.configuration_epoch),
        process_epoch: i64::from(permit.process_epoch),
        reserved_searches: 0,
        reserved_fetches: 0,
        committed_at_ms: now,
    };
    let readback = match permit
        .repository
        .reserve_provider_attempt_v17(command)
        .await
    {
        Ok(readback) => readback,
        Err(RepositoryFault::AlreadyConsumed) => {
            return ProviderQueryAuthorization::Denied {
                reason: AuthorizationDenial::Replay,
            }
        }
        Err(RepositoryFault::InvariantViolation) => {
            return ProviderQueryAuthorization::Denied {
                reason: AuthorizationDenial::MismatchedBinding,
            }
        }
        Err(RepositoryFault::Unavailable | RepositoryFault::CorruptState) => {
            return ProviderQueryAuthorization::Denied {
                reason: AuthorizationDenial::ResolverUnavailable,
            }
        }
        Err(_) => {
            return ProviderQueryAuthorization::Denied {
                reason: AuthorizationDenial::RepositoryFault,
            }
        }
    };
    let attempt = ProviderAttemptIdentity {
        reservation_id: readback.reservation_id,
        attempt_number: operation.attempt_number(),
        operation_hmac: readback.operation_hmac,
    };
    match operation.kind {
        ProviderOperationKind::Search => {
            ProviderQueryAuthorization::AuthorizedSearch(AuthorizedSearch {
                provider: operation.provider,
                query: permit.plan.query.clone().unwrap_or_default(),
                locale: permit.plan.locale,
                attempt,
                plan_hmac: permit.plan.sealed_plan_hmac,
                authorization_id: permit.plan.authorization_id.clone(),
                session_id: permit.plan.session_id.clone(),
                execution_turn_id: permit.plan.execution_turn_id.clone(),
                authorization_hmac: permit.authorization_hmac,
                permit_nonce_hmac: permit.permit_nonce_hmac,
                repository: Arc::clone(&permit.repository),
            })
        }
        ProviderOperationKind::DirectFetch => {
            ProviderQueryAuthorization::AuthorizedDirectFetch(AuthorizedDirectFetch {
                url: permit.plan.normalized_url.clone().unwrap_or_default(),
                attempt,
            })
        }
        ProviderOperationKind::CandidateFetch => {
            ProviderQueryAuthorization::AuthorizedCandidateFetch(AuthorizedCandidateFetch {
                candidate: match operation.candidate {
                    Some(candidate) => candidate,
                    None => {
                        return ProviderQueryAuthorization::Denied {
                            reason: AuthorizationDenial::InvalidOperation,
                        }
                    }
                },
                attempt,
            })
        }
    }
}

fn repository_denial(fault: RepositoryFault) -> AuthorizationDenial {
    match fault {
        RepositoryFault::AlreadyConsumed => AuthorizationDenial::Replay,
        RepositoryFault::InvariantViolation => AuthorizationDenial::MismatchedBinding,
        RepositoryFault::Unavailable | RepositoryFault::CorruptState => {
            AuthorizationDenial::ResolverUnavailable
        }
        RepositoryFault::InvalidInput => AuthorizationDenial::InvalidOperation,
        RepositoryFault::ForeignKeysDisabled
        | RepositoryFault::Storage
        | RepositoryFault::Crypto
        | RepositoryFault::Clock
        | RepositoryFault::ConflictingRetry => AuthorizationDenial::RepositoryFault,
    }
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_preserves_accepted_order() {
        let value = compose_named_query(
            QueryOperation::Comparison,
            "Left",
            QueryKind::Product,
            Some(("Right", QueryKind::Product)),
            QueryFocus::GeneralPublicFacts,
            QueryModifiers::stable_authoritative(),
        )
        .unwrap();
        assert_eq!(
            value,
            r#""Left" versus "Right" product public information comparison"#
        );
    }

    #[test]
    fn urls_are_normalized_without_rewriting_path_or_value_case() {
        assert_eq!(
            normalize_public_url("HTTPS://Example.COM:443/Path?b=Two&utm_source=x&a=One#frag")
                .unwrap(),
            "https://example.com/Path?a=One&b=Two"
        );
    }

    #[test]
    fn closed_operation_templates_and_suffix_order_are_exact() {
        let modifiers = QueryModifiers {
            locale: QueryLocale::Slovak,
            freshness: Freshness::Latest,
            authority: AuthorityPreference::OfficialFirst,
            corroboration: Corroboration::TwoIndependent,
        };
        assert_eq!(
            compose_named_query(
                QueryOperation::Research,
                "Ada Lovelace",
                QueryKind::Person,
                None,
                QueryFocus::GeneralPublicFacts,
                modifiers,
            )
            .unwrap(),
            r#""Ada Lovelace" person public information research latest official"#
        );
        assert_eq!(
            compose_named_query(
                QueryOperation::Specifications,
                "Widget v2",
                QueryKind::Product,
                None,
                QueryFocus::TechnicalSpecification,
                QueryModifiers::stable_authoritative(),
            )
            .unwrap(),
            r#""Widget v2" product technical specifications"#
        );
        assert_eq!(
            compose_named_query(
                QueryOperation::Verification,
                "ISO 8601",
                QueryKind::TechnicalStandard,
                None,
                QueryFocus::LegalInformation,
                QueryModifiers {
                    freshness: Freshness::Today,
                    ..QueryModifiers::stable_authoritative()
                },
            )
            .unwrap(),
            r#""ISO 8601" standard legal information verification today"#
        );
    }

    #[test]
    fn unsupported_focus_kind_pairs_and_credential_urls_fail_closed() {
        assert_eq!(
            compose_named_query(
                QueryOperation::Lookup,
                "weather",
                QueryKind::Product,
                None,
                QueryFocus::Weather,
                QueryModifiers::stable_authoritative(),
            ),
            Err(QueryError::Unsupported)
        );
        assert_eq!(
            normalize_public_url("https://user:password@example.test/"),
            Err(QueryError::InvalidUrl)
        );
    }

    #[test]
    fn direct_url_removes_only_allowed_tracking_keys() {
        assert_eq!(
            normalize_public_url("HTTPS://Example.COM:443/p/A?z=2&utm_source=x&a=Value#fragment")
                .unwrap(),
            "https://example.com/p/A?a=Value&z=2"
        );
        assert_eq!(
            normalize_public_url("https://example.com/?token=secret"),
            Err(QueryError::InvalidUrl)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn admission_commits_one_search_and_returns_content_only_after_commit() {
        use crate::reference_resolution::crypto::{CryptoCustody, FakeKeyProvider};
        use crate::reference_resolution::repository::SqliteRepository;
        use rusqlite::Connection;
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .unwrap();
        crate::embedded::migrations::runner()
            .run(&mut connection)
            .unwrap();
        connection
            .execute(
                "INSERT INTO sessions (id, started_at) VALUES ('1', '2026-08-20T00:00:00Z')",
                [],
            )
            .unwrap();
        let turn_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        connection
            .execute(
                "INSERT INTO reference_turns
                 (turn_id, session_id, chat_session_id, session_seq, origin, state,
                  input_hmac, hmac_key_version, schema_version, grammar_version,
                  normalization_version, compatibility_epoch, created_at_ms,
                  open_expires_at_ms)
                 VALUES (?1, '1', '1', 1, 'chat', 'open', zeroblob(32), 1,
                         1, 1, 1, 1, ?2, ?2 + 3600000)",
                rusqlite::params![turn_id, system_now_ms().saturating_sub(1000)],
            )
            .unwrap();
        let repository = Arc::new(SqliteRepository::new(
            Arc::new(Mutex::new(connection)),
            Arc::new(CryptoCustody::with_provider(
                FakeKeyProvider::deterministic(),
            )),
        ));
        let authorization_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_owned();
        let issued_at_ms = system_now_ms().saturating_sub(1000);
        let (permit, mut operations) = seal_query_plan(
            Arc::clone(&repository),
            SealPlanRequest {
                operation: QueryOperation::Lookup,
                focus: QueryFocus::GeneralPublicFacts,
                modifiers: QueryModifiers::stable_authoritative(),
                referents: AuthorizedReferentSet::Single(AuthorizedReferent {
                    mention_id: "mention".into(),
                    referent_id: "referent".into(),
                    term: "Ada Lovelace".into(),
                    kind: QueryKind::Person,
                    authorization_method: AuthorizationMethod::CurrentUser,
                    language: QueryLocale::English,
                    comparison_side: None,
                }),
                authorization_id: authorization_id.clone(),
                session_id: "1".into(),
                initiating_turn_id: turn_id.into(),
                execution_turn_id: turn_id.into(),
                authorization_method: AuthorizationMethod::CurrentUser,
                provider_scope: "web_search_fetch",
                process_epoch: 1,
                configuration_epoch: 1,
                resolver_mode_epoch: 1,
                issued_at_ms,
            },
        )
        .unwrap();
        let operation = operations.pop().unwrap();
        repository
            .database_for_test()
            .await
            .execute(
                "INSERT INTO query_authorizations
                 (authorization_id, session_id, initiating_turn_id, execution_turn_id,
                  referent_id, authorization_method, provider_scope, query_plan_hmac,
                  permit_nonce_hmac, plan_version, schema_version, grammar_version,
                  normalization_version, hmac_key_version, compatibility_epoch,
                  configuration_epoch, process_epoch, search_budget, fetch_budget,
                  issued_at_ms, expires_at_ms)
                 VALUES (?1, '1', ?2, ?2, 'referent', 'current_user',
                         'web_search_fetch', ?3, ?4, 1, 17, 1, 1, 1, 1, 1, 1,
                         1, 3, ?5, ?5 + 300000)",
                rusqlite::params![
                    authorization_id,
                    turn_id,
                    permit.plan.sealed_plan_hmac.as_slice(),
                    permit.permit_nonce_hmac.as_slice(),
                    issued_at_ms
                ],
            )
            .unwrap();
        repository
            .database_for_test()
            .await
            .execute(
                "INSERT INTO query_authorization_operations
                 (authorization_id, operation_ordinal, operation_hmac,
                  operation_kind, provider, max_attempts)
                 VALUES (?1, 0, ?2, 'search', 'wikipedia', 1)",
                rusqlite::params![
                    permit.plan.authorization_id,
                    operation.operation_hmac.as_slice()
                ],
            )
            .unwrap();

        let operation2 = ProviderOperation {
            slot: operation.slot,
            kind: operation.kind,
            provider: operation.provider,
            operation_hmac: operation.operation_hmac,
            variant_id: operation.variant_id.clone(),
            variant_hmac: operation.variant_hmac,
            attempt_number: operation.attempt_number,
            parent_reservation_id: operation.parent_reservation_id.clone(),
            parent_reservation_hmac: operation.parent_reservation_hmac,
            candidate: None,
            retry_operation_hmac: None,
        };
        let (first, second) = tokio::join!(
            admit_provider_query(&permit, operation),
            admit_provider_query(&permit, operation2)
        );
        assert!(matches!(
            (&first, &second),
            (
                ProviderQueryAuthorization::AuthorizedSearch(_),
                ProviderQueryAuthorization::Denied {
                    reason: AuthorizationDenial::Replay
                }
            ) | (
                ProviderQueryAuthorization::Denied {
                    reason: AuthorizationDenial::Replay
                },
                ProviderQueryAuthorization::AuthorizedSearch(_)
            )
        ));
        let database = repository.database_for_test().await;
        let counters: (i64, i64, i64) = database
            .query_row(
                "SELECT reserved_searches,
                        (SELECT COUNT(*) FROM provider_attempt_reservations_v17),
                        (SELECT COUNT(*) FROM query_authorization_operations
                         WHERE authorization_id=?1 AND reserved_attempts=1)
                 FROM query_authorizations WHERE authorization_id=?1",
                rusqlite::params![permit.plan.authorization_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(counters, (1, 1, 1));
    }
}

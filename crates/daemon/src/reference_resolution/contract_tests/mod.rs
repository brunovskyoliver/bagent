//! Slice 1 verification contracts.
//!
//! This module is compiled only by the daemon test target. It contains static,
//! content-free expectations and does not implement reference resolution.

#![allow(unexpected_cfgs)]

pub(crate) mod pure {
    include!("pure.rs");
}

pub(crate) mod bypass {
    include!("bypass.rs");
}

pub(crate) mod automation {
    include!("automation.rs");
}

pub(crate) mod persistence {
    include!("persistence.rs");
}

pub(crate) mod producers {
    include!("producers.rs");
}

pub(crate) mod confirmation {
    include!("confirmation.rs");
}

pub(crate) mod query {
    use crate::reference_resolution::{
        compose_query_for_test, normalize_public_term_for_test, QueryFocus, QueryKind,
        QueryModifiers, QueryOperation, QueryReferentInput,
    };

    #[test]
    fn research_query_composition_is_deterministic_and_quotes_the_term() {
        let query = compose_query_for_test(
            QueryOperation::Research,
            QueryReferentInput::named("Aster   Nova\\12", QueryKind::Product),
            QueryFocus::Price,
            QueryModifiers::stable_authoritative(),
        )
        .expect("synthetic query plan");
        assert_eq!(query, r#""Aster Nova\\12" product price research"#);
    }

    #[test]
    fn term_normalization_preserves_case_accents_and_internal_punctuation() {
        assert_eq!(
            normalize_public_term_for_test("  Žluť  Kůň/v2  "),
            "Žluť Kůň/v2"
        );
    }

    fn impossible<T>() -> T {
        panic!("compile-fail fixture value")
    }

    #[cfg(reference_compilefail_fixture = "slice9_raw_string_search")]
    fn slice9_raw_string_search() {
        let value = String::from("raw");
        fn typed(_: crate::reference_resolution::AuthorizedSearch) {}
        typed(value); // Slice 9 fixture: raw String cannot call typed search.
    }

    #[cfg(reference_compilefail_fixture = "slice9_str_search")]
    fn slice9_str_search() {
        let value = "raw";
        fn typed(_: crate::reference_resolution::AuthorizedSearch) {}
        typed(value); // Slice 9 fixture: &str cannot call typed search.
    }

    #[cfg(reference_compilefail_fixture = "slice9_raw_url_direct_fetch")]
    fn slice9_raw_url_direct_fetch() {
        let value = url::Url::parse("https://example.test/").unwrap();
        fn typed(_: crate::reference_resolution::AuthorizedDirectFetch) {}
        typed(value); // Slice 9 fixture: raw Url cannot call direct fetch.
    }

    #[cfg(reference_compilefail_fixture = "slice9_web_candidate_fetch")]
    fn slice9_web_candidate_fetch() {
        let value: crate::evidence::WebCandidate = impossible();
        fn typed(_: crate::reference_resolution::AuthorizedCandidateFetch) {}
        typed(value); // Slice 9 fixture: ordinary WebCandidate cannot fetch.
    }

    #[cfg(reference_compilefail_fixture = "slice9_permit_constructor")]
    fn slice9_permit_constructor() {
        let _ = crate::reference_resolution::ProviderQueryPermit::new(); // Slice 9 fixture: permit constructor is private.
    }

    #[cfg(reference_compilefail_fixture = "slice9_authorized_constructor")]
    fn slice9_authorized_constructor() {
        let _ = crate::reference_resolution::AuthorizedSearch::new(); // Slice 9 fixture: authorized constructor is private.
    }

    #[cfg(reference_compilefail_fixture = "slice9_capability_clone")]
    fn slice9_capability_clone() {
        let value: crate::reference_resolution::ProviderQueryPermit = impossible();
        let _ = value.clone(); // Slice 9 fixture: capability cannot clone.
    }

    #[cfg(reference_compilefail_fixture = "slice9_capability_serialize")]
    fn slice9_capability_serialize() {
        let value: crate::reference_resolution::ProviderQueryPermit = impossible();
        let _ = serde_json::to_string(&value); // Slice 9 fixture: capability cannot serialize.
    }

    #[cfg(reference_compilefail_fixture = "slice9_capability_deserialize")]
    fn slice9_capability_deserialize() {
        type Permit = crate::reference_resolution::ProviderQueryPermit;
        let _ = serde_json::from_str::<Permit>("{}"); // Slice 9 fixture: capability cannot deserialize.
    }

    #[cfg(reference_compilefail_fixture = "slice9_capability_display")]
    fn slice9_capability_display() {
        let value: crate::reference_resolution::ProviderQueryPermit = impossible();
        let _ = format!("{value}"); // Slice 9 fixture: capability cannot display.
    }

    #[cfg(reference_compilefail_fixture = "slice9_capability_copy")]
    fn slice9_capability_copy() {
        let value: crate::reference_resolution::ProviderQueryPermit = impossible();
        let _copy = value;
        let _copy_again = value; // Slice 9 fixture: capability cannot copy.
    }

    #[cfg(reference_compilefail_fixture = "slice9_moved_operation")]
    fn slice9_moved_operation() {
        let value: crate::reference_resolution::AuthorizedSearch = impossible();
        fn typed(_: crate::reference_resolution::AuthorizedSearch) {}
        typed(value);
        typed(value); // Slice 9 fixture: moved authorized operation cannot be reused.
    }

    #[cfg(reference_compilefail_fixture = "slice9_search_as_fetch")]
    fn slice9_search_as_fetch() {
        let value: crate::reference_resolution::AuthorizedSearch = impossible();
        fn typed(_: crate::reference_resolution::AuthorizedCandidateFetch) {}
        typed(value); // Slice 9 fixture: search authorization cannot fetch.
    }

    #[cfg(reference_compilefail_fixture = "slice9_candidate_forge")]
    fn slice9_candidate_forge() {
        let _ = crate::reference_resolution::SealedDiscoveredCandidate {}; // Slice 9 fixture: candidate cannot be forged.
    }

    #[cfg(reference_compilefail_fixture = "slice9_direct_as_search")]
    fn slice9_direct_as_search() {
        let value: crate::reference_resolution::AuthorizedDirectFetch = impossible();
        fn typed(_: crate::reference_resolution::AuthorizedSearch) {}
        typed(value); // Slice 9 fixture: direct authorization cannot become search.
    }
}

pub(crate) mod harness {
    use std::sync::{Arc, Mutex};

    const TRACE_SCHEMA_VERSION: u16 = 1;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CaseId {
        RegistryComplete,
        StructuralTrace,
        TeardownComplete,
        PrivacyClosedTypes,
    }

    impl CaseId {
        const ALL: [Self; 4] = [
            Self::RegistryComplete,
            Self::StructuralTrace,
            Self::TeardownComplete,
            Self::PrivacyClosedTypes,
        ];

        const fn as_str(self) -> &'static str {
            match self {
                Self::RegistryComplete => "slice1.registry.complete",
                Self::StructuralTrace => "slice1.trace.structural",
                Self::TeardownComplete => "slice1.teardown.complete",
                Self::PrivacyClosedTypes => "slice1.privacy.closed_types",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum StructuralOutcome {
        RegistryComplete,
        TraceMatched,
        TeardownComplete,
        PrivacySafe,
    }

    impl StructuralOutcome {
        const fn as_str(self) -> &'static str {
            match self {
                Self::RegistryComplete => "registry_complete",
                Self::TraceMatched => "trace_matched",
                Self::TeardownComplete => "teardown_complete",
                Self::PrivacySafe => "privacy_safe",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum OperationClass {
        Registry,
        Recorder,
        Teardown,
        PrivacyScan,
        PromptConstruction,
        ModelInference,
        MailAccess,
        ProviderTransport,
        RuntimeMutation,
        ResolverRuntime,
        ResolverTask,
        ResolverRoute,
        ResolverEvent,
        ResolverDiagnostic,
        ResolverRepository,
        ResolverCrypto,
        ResolverConfirmation,
        ResolverAuthorization,
        ResolverAdmission,
        ResolverModel,
        ResolverMail,
        ResolverTool,
        ResolverProvider,
    }

    impl OperationClass {
        const fn as_str(self) -> &'static str {
            match self {
                Self::Registry => "registry",
                Self::Recorder => "recorder",
                Self::Teardown => "teardown",
                Self::PrivacyScan => "privacy_scan",
                Self::PromptConstruction => "prompt_construction",
                Self::ModelInference => "model_inference",
                Self::MailAccess => "mail_access",
                Self::ProviderTransport => "provider_transport",
                Self::RuntimeMutation => "runtime_mutation",
                Self::ResolverRuntime => "resolver_runtime",
                Self::ResolverTask => "resolver_task",
                Self::ResolverRoute => "resolver_route",
                Self::ResolverEvent => "resolver_event",
                Self::ResolverDiagnostic => "resolver_diagnostic",
                Self::ResolverRepository => "resolver_repository",
                Self::ResolverCrypto => "resolver_crypto",
                Self::ResolverConfirmation => "resolver_confirmation",
                Self::ResolverAuthorization => "resolver_authorization",
                Self::ResolverAdmission => "resolver_admission",
                Self::ResolverModel => "resolver_model",
                Self::ResolverMail => "resolver_mail",
                Self::ResolverTool => "resolver_tool",
                Self::ResolverProvider => "resolver_provider",
            }
        }
    }

    const RESOLVER_FORBIDDEN_CALLS: &[OperationClass] = &[
        OperationClass::ResolverRuntime,
        OperationClass::ResolverTask,
        OperationClass::ResolverRoute,
        OperationClass::ResolverEvent,
        OperationClass::ResolverDiagnostic,
        OperationClass::ResolverRepository,
        OperationClass::ResolverCrypto,
        OperationClass::ResolverConfirmation,
        OperationClass::ResolverAuthorization,
        OperationClass::ResolverAdmission,
        OperationClass::ResolverModel,
        OperationClass::ResolverMail,
        OperationClass::ResolverTool,
        OperationClass::ResolverProvider,
    ];

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ResultClass {
        Matched,
        Absent,
    }

    impl ResultClass {
        const fn as_str(self) -> &'static str {
            match self {
                Self::Matched => "matched",
                Self::Absent => "absent",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CompletionClass {
        Success,
        Failure,
    }

    impl CompletionClass {
        const fn as_str(self) -> &'static str {
            match self {
                Self::Success => "success",
                Self::Failure => "failure",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum OracleMutationId {
        MissingRequiredCase,
        WrongStructuralTrace,
        TeardownFailure,
        ForbiddenPrivacyField,
    }

    impl OracleMutationId {
        const fn as_str(self) -> &'static str {
            match self {
                Self::MissingRequiredCase => "missing_required_case",
                Self::WrongStructuralTrace => "wrong_structural_trace",
                Self::TeardownFailure => "teardown_failure",
                Self::ForbiddenPrivacyField => "forbidden_privacy_field",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct CaseContract {
        id: CaseId,
        suite_version: u16,
        schema_version: u16,
        expected_outcome: StructuralOutcome,
        expected_calls: &'static [OperationClass],
        forbidden_calls: &'static [OperationClass],
        teardown_required: bool,
        oracle_mutation: OracleMutationId,
    }

    const FORBIDDEN_CALLS: &[OperationClass] = &[
        OperationClass::PromptConstruction,
        OperationClass::ModelInference,
        OperationClass::MailAccess,
        OperationClass::ProviderTransport,
        OperationClass::RuntimeMutation,
    ];

    const CASES: [CaseContract; 4] = [
        CaseContract {
            id: CaseId::RegistryComplete,
            suite_version: 1,
            schema_version: 1,
            expected_outcome: StructuralOutcome::RegistryComplete,
            expected_calls: &[OperationClass::Registry],
            forbidden_calls: FORBIDDEN_CALLS,
            teardown_required: false,
            oracle_mutation: OracleMutationId::MissingRequiredCase,
        },
        CaseContract {
            id: CaseId::StructuralTrace,
            suite_version: 1,
            schema_version: 1,
            expected_outcome: StructuralOutcome::TraceMatched,
            expected_calls: &[OperationClass::Recorder],
            forbidden_calls: FORBIDDEN_CALLS,
            teardown_required: false,
            oracle_mutation: OracleMutationId::WrongStructuralTrace,
        },
        CaseContract {
            id: CaseId::TeardownComplete,
            suite_version: 1,
            schema_version: 1,
            expected_outcome: StructuralOutcome::TeardownComplete,
            expected_calls: &[OperationClass::Teardown],
            forbidden_calls: FORBIDDEN_CALLS,
            teardown_required: true,
            oracle_mutation: OracleMutationId::TeardownFailure,
        },
        CaseContract {
            id: CaseId::PrivacyClosedTypes,
            suite_version: 1,
            schema_version: 1,
            expected_outcome: StructuralOutcome::PrivacySafe,
            expected_calls: &[OperationClass::PrivacyScan],
            forbidden_calls: FORBIDDEN_CALLS,
            teardown_required: false,
            oracle_mutation: OracleMutationId::ForbiddenPrivacyField,
        },
    ];

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TraceRecord {
        schema_version: u16,
        sequence: u32,
        causal_group: u16,
        attempt_ordinal: u16,
        operation: OperationClass,
        structural_result: ResultClass,
        completion: CompletionClass,
    }

    #[derive(Clone, Default)]
    pub(super) struct BoundaryOperationRecorder {
        records: Arc<Mutex<Vec<TraceRecord>>>,
    }

    impl BoundaryOperationRecorder {
        fn record(
            &self,
            causal_group: u16,
            attempt_ordinal: u16,
            operation: OperationClass,
            structural_result: ResultClass,
            completion: CompletionClass,
        ) {
            let mut records = self.records.lock().expect("recorder lock");
            let sequence = u32::try_from(records.len() + 1).expect("bounded trace");
            records.push(TraceRecord {
                schema_version: TRACE_SCHEMA_VERSION,
                sequence,
                causal_group,
                attempt_ordinal,
                operation,
                structural_result,
                completion,
            });
        }

        fn snapshot(&self) -> Vec<TraceRecord> {
            self.records.lock().expect("recorder lock").clone()
        }

        pub(super) fn record_resolver_operation(&self) {
            self.record(
                0,
                1,
                OperationClass::ResolverRuntime,
                ResultClass::Matched,
                CompletionClass::Success,
            );
        }

        pub(super) fn is_empty(&self) -> bool {
            self.records.lock().expect("recorder lock").is_empty()
        }

        pub(super) fn assert_no_resolver_operations(&self) {
            let records = self.snapshot();
            for forbidden in RESOLVER_FORBIDDEN_CALLS {
                assert!(
                    records.iter().all(|record| record.operation != *forbidden),
                    "unexpected resolver operation: {}",
                    forbidden.as_str()
                );
            }
        }
    }

    fn contract(id: CaseId) -> &'static CaseContract {
        CASES
            .iter()
            .find(|contract| contract.id == id)
            .expect("registered Slice 1 case")
    }

    fn emit_case_result(id: CaseId) {
        let case = contract(id);
        println!(
            "REFERENCE_CASE_RESULT case={} outcome={}",
            case.id.as_str(),
            case.expected_outcome.as_str()
        );
    }

    #[test]
    fn registered_cases_are_complete() {
        assert_eq!(CASES.len(), CaseId::ALL.len());
        for (case, expected_id) in CASES.iter().zip(CaseId::ALL) {
            assert_eq!(case.id, expected_id);
            assert_eq!(case.suite_version, 1);
            assert_eq!(case.schema_version, 1);
            assert!(!case.expected_calls.is_empty());
            assert_eq!(case.forbidden_calls, FORBIDDEN_CALLS);
            println!(
                "REFERENCE_REGISTRY case={} outcome={} teardown={} mutation={}",
                case.id.as_str(),
                case.expected_outcome.as_str(),
                u8::from(case.teardown_required),
                case.oracle_mutation.as_str()
            );
        }
        emit_case_result(CaseId::RegistryComplete);
    }

    #[test]
    fn recorder_emits_expected_structural_trace() {
        let recorder = BoundaryOperationRecorder::default();
        recorder.record(
            0,
            1,
            OperationClass::Recorder,
            ResultClass::Matched,
            CompletionClass::Success,
        );
        let records = recorder.snapshot();
        assert_eq!(
            records,
            vec![TraceRecord {
                schema_version: 1,
                sequence: 1,
                causal_group: 0,
                attempt_ordinal: 1,
                operation: OperationClass::Recorder,
                structural_result: ResultClass::Matched,
                completion: CompletionClass::Success,
            }]
        );
        let record = records[0];
        println!(
                "REFERENCE_TRACE case={} schema={} sequence={} causal_group={} attempt={} operation={} structural_result={} completion={}",
                CaseId::StructuralTrace.as_str(),
                record.schema_version,
                record.sequence,
                record.causal_group,
                record.attempt_ordinal,
                record.operation.as_str(),
                record.structural_result.as_str(),
                record.completion.as_str()
            );
        for forbidden in FORBIDDEN_CALLS {
            let count = records
                .iter()
                .filter(|record| record.operation == *forbidden)
                .count();
            println!(
                "REFERENCE_ZERO_CALL class={} count={}",
                forbidden.as_str(),
                count
            );
        }
        emit_case_result(CaseId::StructuralTrace);
    }

    #[test]
    fn temporary_artifact_teardown_reaches_completion_marker() {
        let temporary = tempfile::Builder::new()
            .prefix("bagent-reference-slice1-")
            .tempdir()
            .expect("create isolated temporary directory");
        let temporary_path = temporary.path().to_path_buf();
        let marker = temporary.path().join("synthetic-artifact");
        std::fs::write(&marker, b"REFERENCE_SYNTHETIC_SAFE_V1\n")
            .expect("write synthetic artifact");
        std::fs::remove_file(&marker).expect("remove synthetic artifact");
        temporary
            .close()
            .expect("remove isolated temporary directory");
        assert!(!temporary_path.exists());
        println!(
            "REFERENCE_TEARDOWN case={} complete=1",
            CaseId::TeardownComplete.as_str()
        );
        emit_case_result(CaseId::TeardownComplete);
    }

    #[test]
    fn recorder_api_is_content_free_and_closed() {
        let recorder = BoundaryOperationRecorder::default();
        recorder.record(
            0,
            1,
            OperationClass::PrivacyScan,
            ResultClass::Absent,
            CompletionClass::Success,
        );
        let rendered = format!("{:?}", recorder.snapshot());
        for forbidden in ["prompt", "query", "url", "path", "identifier", "content"] {
            assert!(!rendered.to_ascii_lowercase().contains(forbidden));
        }
        emit_case_result(CaseId::PrivacyClosedTypes);
    }

    // compilefail:synthetic_type_mismatch
    #[cfg(reference_compilefail_fixture = "synthetic_type_mismatch")]
    const SYNTHETIC_TYPE_MISMATCH: u8 = "closed-enum-required";

    #[test]
    fn closed_failure_category_is_constructible_for_future_controls() {
        let value = CompletionClass::Failure;
        assert_eq!(value.as_str(), "failure");
    }

    // compilefail:producer_constructor_privacy
    #[cfg(reference_compilefail_fixture = "producer_constructor_privacy")]
    fn PRODUCER_CONSTRUCTOR_PRIVACY() {
        crate::reference_resolution::artifacts::CanonicalWebArtifact::private_constructor_for_internal();
    }

    // compilefail:producer_witness_reuse
    #[cfg(reference_compilefail_fixture = "producer_witness_reuse")]
    fn PRODUCER_WITNESS_REUSE() {
        let mut execution = crate::reference_resolution::artifacts::begin_turn(
            crate::reference_resolution::TurnId::new(),
            crate::reference_resolution::SessionId::new("synthetic").unwrap(),
            crate::reference_resolution::TurnOrigin::Chat,
        );
        let witness = execution.deterministic_terminal_witness();
        let _ = crate::reference_resolution::artifacts::finish_no_mention(
            witness,
            crate::reference_resolution::artifacts::ClosedNoMentionReason::Blocked,
        );
        let witness_reused = witness;
    }

    // compilefail:producer_witness_clone
    #[cfg(reference_compilefail_fixture = "producer_witness_clone")]
    fn PRODUCER_WITNESS_CLONE() {
        let mut execution = crate::reference_resolution::artifacts::begin_turn(
            crate::reference_resolution::TurnId::new(),
            crate::reference_resolution::SessionId::new("synthetic").unwrap(),
            crate::reference_resolution::TurnOrigin::Chat,
        );
        let witness = execution.deterministic_terminal_witness();
        let _ = witness.clone();
    }

    // compilefail:producer_cross_producer
    #[cfg(reference_compilefail_fixture = "producer_cross_producer")]
    fn PRODUCER_CROSS_PRODUCER() {
        let mut execution = crate::reference_resolution::artifacts::begin_turn(
            crate::reference_resolution::TurnId::new(),
            crate::reference_resolution::SessionId::new("synthetic").unwrap(),
            crate::reference_resolution::TurnOrigin::Chat,
        );
        let deterministic_witness = execution.deterministic_terminal_witness();
        let _ = crate::reference_resolution::artifacts::finish_typed_mail(
            deterministic_witness,
            todo!(),
            todo!(),
            todo!(),
            todo!(),
        );
    }
}

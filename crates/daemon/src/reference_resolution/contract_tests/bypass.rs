use super::super::{
    parse_resolver_mode_with_status, production, select_runtime, ConversationalReferenceResolver,
    ResolverFault, ResolverMode, ResolverModeParseStatus, RuntimeSelection,
};
use super::harness::BoundaryOperationRecorder;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn resolver_factory(
    _mode: ResolverMode,
) -> Result<Arc<dyn ConversationalReferenceResolver>, ResolverFault> {
    Ok(production())
}

#[test]
fn subordinate_off_does_not_invoke_panicking_resolver_factory() {
    let selection = select_runtime(
        || Some("off".to_owned()),
        |_mode| -> Result<Arc<dyn ConversationalReferenceResolver>, ResolverFault> {
            panic!("resolver factory was invoked")
        },
    );

    assert_eq!(selection.selection_label(), "resolver_off");
    assert_eq!(selection.parse_status_label(), "valid");
    assert!(selection.into_runtime().is_none());
}

#[test]
fn invalid_subordinate_configuration_fails_closed_before_construction() {
    let selection = select_runtime(
        || Some("Enforce".to_owned()),
        |_mode| -> Result<Arc<dyn ConversationalReferenceResolver>, ResolverFault> {
            panic!("invalid subordinate configuration reached construction")
        },
    );

    assert_eq!(selection.selection_label(), "resolver_off");
    assert_eq!(selection.parse_status_label(), "invalid");
    assert!(selection.into_runtime().is_none());
}

#[test]
fn subordinate_off_stores_no_app_state_runtime_handle() {
    let startup = include_str!("../../main.rs");
    assert!(startup.contains(
        "resolver_runtime: Option<reference_resolution::ResolverRuntime>"
    ));
    assert!(startup.contains("let resolver_runtime = resolver_selection.into_runtime();"));
    assert!(startup.contains("        resolver_runtime,"));

    for subordinate in [Some("off"), Some("Enforce"), None] {
        let selection =
            select_runtime(|| subordinate.map(str::to_owned), resolver_factory);
        let resolver_runtime = selection.into_runtime();
        assert!(resolver_runtime.is_none());
    }
}

#[test]
fn absent_and_invalid_subordinate_modes_select_off_without_construction() {
    for value in [
        None,
        Some(""),
        Some("unknown"),
        Some("OFF"),
        Some(" observe"),
        Some("observe "),
        Some("fixture_enforcement"),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory_calls = Arc::clone(&calls);
        let selection = select_runtime(
            move || value.map(str::to_owned),
            move |_mode| {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                resolver_factory(ResolverMode::Observe)
            },
        );

        assert_eq!(selection.selection_label(), "resolver_off");
        assert_eq!(
            selection.parse_status_label(),
            if value.is_none() { "absent" } else { "invalid" }
        );
        assert!(selection.into_runtime().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn each_enabled_subordinate_mode_constructs_the_inert_resolver_once() {
    for (value, expected_mode) in [
        ("persistence", ResolverMode::Persistence),
        ("observe", ResolverMode::Observe),
        ("enforce", ResolverMode::Enforce),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory_calls = Arc::clone(&calls);
        let selection = select_runtime(
            move || Some(value.to_owned()),
            move |mode| {
                assert_eq!(mode, expected_mode);
                factory_calls.fetch_add(1, Ordering::SeqCst);
                resolver_factory(mode)
            },
        );

        assert_eq!(selection.selection_label(), "resolver_enabled");
        assert_eq!(selection.mode_label(), value);
        let runtime = selection.into_runtime().expect("enabled mode runtime");
        assert_eq!(runtime.mode(), expected_mode);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn resolver_construction_failure_is_unavailable_not_legacy() {
    let selection = select_runtime(
        || Some("observe".to_owned()),
        |_mode| -> Result<Arc<dyn ConversationalReferenceResolver>, ResolverFault> {
            Err(ResolverFault::Unavailable)
        },
    );

    assert_eq!(selection.selection_label(), "resolver_unavailable");
    assert_eq!(selection.mode_label(), "observe");
    assert_eq!(selection.parse_status_label(), "valid");
    assert!(selection.into_runtime().is_none());
}

#[test]
fn migration_execution_precedes_resolver_selection_in_daemon_startup() {
    let source = include_str!("../../main.rs");
    let migration = source
        .find("embedded::migrations::runner()")
        .expect("unconditional migration runner");
    let selection = source
        .find("reference_resolution::select_runtime(")
        .expect("resolver startup selection");
    let subordinate = source
        .find("REFERENCE_RESOLVER_MODE_ENV")
        .expect("subordinate setting supplier");

    assert!(migration < selection);
    assert!(selection < subordinate);
}

#[test]
fn invalid_mode_status_is_content_free() {
    let sentinel = "invalid-private-reference-sentinel";
    let parsed = parse_resolver_mode_with_status(Some(sentinel));
    assert_eq!(parsed.status(), ResolverModeParseStatus::Invalid);
    assert_eq!(parsed.mode(), ResolverMode::Off);
    assert!(!format!("{parsed:?}").contains(sentinel));

    let selection = select_runtime(
        || Some(sentinel.to_owned()),
        resolver_factory,
    );
    assert!(!format!("{selection:?}").contains(sentinel));
}

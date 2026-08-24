-- V17: restart-safe provider authorization continuation and HMAC-only execution.
-- V16 execution rows remain installed but dormant. No V16 row is copied or
-- backfilled into this representation.

CREATE TABLE reference_confirmation_continuations_v17 (
    confirmation_id TEXT PRIMARY KEY NOT NULL
        REFERENCES reference_confirmation_tombstones(confirmation_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    initiating_turn_id TEXT NOT NULL,
    continuation_ciphertext BLOB NOT NULL CHECK (length(continuation_ciphertext) > 0),
    continuation_hmac BLOB NOT NULL CHECK (length(continuation_hmac) = 32),
    sealed_plan_hmac BLOB NOT NULL CHECK (length(sealed_plan_hmac) = 32),
    referent_set_hmac BLOB NOT NULL CHECK (length(referent_set_hmac) = 32),
    capability_version INTEGER NOT NULL CHECK (capability_version > 0),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    grammar_version INTEGER NOT NULL CHECK (grammar_version > 0),
    normalization_version INTEGER NOT NULL CHECK (normalization_version > 0),
    provider_scope TEXT NOT NULL CHECK (provider_scope IN ('web_search_fetch', 'direct_public_fetch')),
    format_version INTEGER NOT NULL CHECK (format_version > 0),
    encryption_key_version INTEGER NOT NULL CHECK (encryption_key_version > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms = created_at_ms + 300000)
);

CREATE INDEX reference_confirmation_continuations_session_expiry_v17
    ON reference_confirmation_continuations_v17(session_id, expires_at_ms);
CREATE INDEX reference_confirmation_continuations_plan_v17
    ON reference_confirmation_continuations_v17(sealed_plan_hmac);

CREATE TRIGGER reference_confirmation_continuations_validate_v17
BEFORE INSERT ON reference_confirmation_continuations_v17
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM reference_confirmation_tombstones confirmation
        WHERE confirmation.confirmation_id = NEW.confirmation_id
          AND confirmation.session_id = NEW.session_id
          AND confirmation.initiating_turn_id = NEW.initiating_turn_id
          AND confirmation.expires_at_ms = NEW.expires_at_ms
    ) THEN RAISE(ABORT, 'continuation parent binding is invalid') END;
END;

CREATE TRIGGER reference_confirmation_continuations_immutable_v17
BEFORE UPDATE ON reference_confirmation_continuations_v17
BEGIN
    SELECT RAISE(ABORT, 'confirmation continuation is immutable');
END;

CREATE TABLE query_operation_variant_sets_v17 (
    variant_set_id TEXT PRIMARY KEY NOT NULL,
    authorization_id TEXT NOT NULL
        REFERENCES query_authorizations(authorization_id) ON DELETE CASCADE,
    operation_slot INTEGER NOT NULL CHECK (operation_slot = 1),
    variant_set_hmac BLOB NOT NULL CHECK (length(variant_set_hmac) = 32),
    state TEXT NOT NULL CHECK (state IN ('sealing', 'ready', 'spent', 'closed')),
    winner_variant_id TEXT,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    UNIQUE (authorization_id, operation_slot)
);

CREATE TABLE query_operation_variants_v17 (
    variant_id TEXT PRIMARY KEY NOT NULL,
    variant_set_id TEXT NOT NULL
        REFERENCES query_operation_variant_sets_v17(variant_set_id) ON DELETE CASCADE,
    variant_kind TEXT NOT NULL CHECK (variant_kind IN (
        'same_provider_retry', 'fallback_provider', 'diversification_query'
    )),
    variant_hmac BLOB NOT NULL CHECK (length(variant_hmac) = 32),
    provider_hmac BLOB NOT NULL CHECK (length(provider_hmac) = 32),
    operation_hmac BLOB NOT NULL CHECK (length(operation_hmac) = 32),
    parent_search_operation_hmac BLOB NOT NULL CHECK (length(parent_search_operation_hmac) = 32),
    attempt_number INTEGER NOT NULL CHECK (attempt_number = 2),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    UNIQUE (variant_set_id, variant_kind),
    UNIQUE (variant_set_id, variant_hmac)
);

CREATE INDEX query_operation_variants_set_v17
    ON query_operation_variants_v17(variant_set_id);
CREATE INDEX query_operation_variants_hmac_v17
    ON query_operation_variants_v17(variant_hmac);

CREATE TRIGGER query_operation_variant_sets_ready_v17
BEFORE UPDATE OF state ON query_operation_variant_sets_v17
WHEN NEW.state = 'ready'
BEGIN
    SELECT CASE WHEN (
        SELECT COUNT(*) FROM query_operation_variants_v17 variant
        WHERE variant.variant_set_id = NEW.variant_set_id
    ) <> 3 THEN RAISE(ABORT, 'variant set must contain exactly three variants') END;
    SELECT CASE WHEN (
        SELECT COUNT(DISTINCT variant.parent_search_operation_hmac)
        FROM query_operation_variants_v17 variant
        WHERE variant.variant_set_id = NEW.variant_set_id
    ) <> 1 THEN RAISE(ABORT, 'variant parents must match') END;
    SELECT CASE WHEN (
        SELECT COUNT(DISTINCT variant.variant_kind)
        FROM query_operation_variants_v17 variant
        WHERE variant.variant_set_id = NEW.variant_set_id
    ) <> 3 THEN RAISE(ABORT, 'variant kinds must be distinct') END;
END;

CREATE TRIGGER query_operation_variant_sets_immutable_v17
BEFORE UPDATE ON query_operation_variants_v17
BEGIN
    SELECT RAISE(ABORT, 'operation variant is immutable');
END;

CREATE TABLE provider_attempt_reservations_v17 (
    reservation_id TEXT PRIMARY KEY NOT NULL,
    authorization_id TEXT NOT NULL
        REFERENCES query_authorizations(authorization_id) ON DELETE CASCADE,
    authorization_hmac BLOB NOT NULL CHECK (length(authorization_hmac) = 32),
    operation_slot INTEGER NOT NULL CHECK (operation_slot >= 0),
    variant_id TEXT REFERENCES query_operation_variants_v17(variant_id) ON DELETE RESTRICT,
    variant_hmac BLOB CHECK (variant_hmac IS NULL OR length(variant_hmac) = 32),
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    parent_reservation_id TEXT,
    parent_reservation_hmac BLOB
        CHECK (parent_reservation_hmac IS NULL OR length(parent_reservation_hmac) = 32),
    candidate_binding_id TEXT,
    candidate_binding_hmac BLOB
        CHECK (candidate_binding_hmac IS NULL OR length(candidate_binding_hmac) = 32),
    provider_hmac BLOB NOT NULL CHECK (length(provider_hmac) = 32),
    operation_hmac BLOB NOT NULL CHECK (length(operation_hmac) = 32),
    sealed_plan_hmac BLOB NOT NULL CHECK (length(sealed_plan_hmac) = 32),
    permit_nonce_hmac BLOB NOT NULL CHECK (length(permit_nonce_hmac) = 32),
    committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms >= 0),
    reserved_searches INTEGER NOT NULL CHECK (reserved_searches BETWEEN 0 AND 2),
    reserved_fetches INTEGER NOT NULL CHECK (reserved_fetches BETWEEN 0 AND 5),
    state TEXT NOT NULL CHECK (state = 'committed'),
    UNIQUE (authorization_id, operation_slot, variant_id, attempt_number)
);

CREATE INDEX provider_attempt_reservations_v17_authorization
    ON provider_attempt_reservations_v17(authorization_id);
CREATE INDEX provider_attempt_reservations_v17_operation
    ON provider_attempt_reservations_v17(operation_hmac);
CREATE INDEX provider_attempt_reservations_v17_candidate
    ON provider_attempt_reservations_v17(candidate_binding_id);
CREATE INDEX provider_attempt_reservations_v17_committed
    ON provider_attempt_reservations_v17(committed_at_ms);
CREATE UNIQUE INDEX provider_attempt_reservations_v17_primary_attempt
    ON provider_attempt_reservations_v17(authorization_id, operation_slot, attempt_number)
    WHERE variant_id IS NULL;

CREATE TRIGGER provider_attempt_reservations_validate_v17
BEFORE INSERT ON provider_attempt_reservations_v17
BEGIN
    SELECT CASE WHEN NEW.attempt_number = 2 AND NEW.variant_id IS NULL
        THEN RAISE(ABORT, 'second attempt requires a selected variant') END;
    SELECT CASE WHEN NEW.operation_slot > 1 AND NEW.parent_reservation_id IS NULL
        THEN RAISE(ABORT, 'fetch requires a parent search reservation') END;
    SELECT CASE WHEN NEW.candidate_binding_id IS NOT NULL AND NEW.candidate_binding_hmac IS NULL
        THEN RAISE(ABORT, 'candidate binding hmac is required') END;
    SELECT CASE WHEN NEW.parent_reservation_id IS NOT NULL AND NEW.parent_reservation_hmac IS NULL
        THEN RAISE(ABORT, 'parent reservation hmac is required') END;
END;

CREATE TRIGGER provider_attempt_reservations_immutable_v17
BEFORE UPDATE ON provider_attempt_reservations_v17
BEGIN
    SELECT RAISE(ABORT, 'provider reservation is immutable');
END;

CREATE TABLE dynamic_candidate_bindings_v17 (
    candidate_binding_id TEXT PRIMARY KEY NOT NULL,
    authorization_id TEXT NOT NULL
        REFERENCES query_authorizations(authorization_id) ON DELETE CASCADE,
    authorization_hmac BLOB NOT NULL CHECK (length(authorization_hmac) = 32),
    fetch_slot INTEGER NOT NULL CHECK (fetch_slot BETWEEN 0 AND 4),
    parent_reservation_id TEXT NOT NULL
        REFERENCES provider_attempt_reservations_v17(reservation_id) ON DELETE CASCADE,
    parent_reservation_hmac BLOB NOT NULL CHECK (length(parent_reservation_hmac) = 32),
    discovery_provider_hmac BLOB NOT NULL CHECK (length(discovery_provider_hmac) = 32),
    normalized_url_hmac BLOB NOT NULL CHECK (length(normalized_url_hmac) = 32),
    source_identity_hmac BLOB NOT NULL CHECK (length(source_identity_hmac) = 32),
    candidate_capability_hmac BLOB NOT NULL CHECK (length(candidate_capability_hmac) = 32),
    retry_relationship_hmac BLOB NOT NULL CHECK (length(retry_relationship_hmac) = 32),
    result_ordinal INTEGER NOT NULL CHECK (result_ordinal >= 0),
    binding_hmac BLOB NOT NULL CHECK (length(binding_hmac) = 32),
    state TEXT NOT NULL CHECK (state IN ('active', 'spent', 'invalidated')),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms >= created_at_ms),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    UNIQUE (authorization_id, fetch_slot, result_ordinal, binding_hmac)
);

CREATE INDEX dynamic_candidate_bindings_v17_authorization
    ON dynamic_candidate_bindings_v17(authorization_id);
CREATE INDEX dynamic_candidate_bindings_v17_parent
    ON dynamic_candidate_bindings_v17(parent_reservation_id);
CREATE INDEX dynamic_candidate_bindings_v17_binding
    ON dynamic_candidate_bindings_v17(binding_hmac);
CREATE INDEX dynamic_candidate_bindings_v17_expiry
    ON dynamic_candidate_bindings_v17(expires_at_ms);

CREATE TRIGGER dynamic_candidate_bindings_validate_v17
BEFORE INSERT ON dynamic_candidate_bindings_v17
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM provider_attempt_reservations_v17 reservation
        WHERE reservation.reservation_id = NEW.parent_reservation_id
          AND reservation.authorization_id = NEW.authorization_id
          AND reservation.operation_slot = 0
          AND reservation.state = 'committed'
    ) THEN RAISE(ABORT, 'candidate parent reservation is invalid') END;
END;

CREATE TRIGGER dynamic_candidate_bindings_immutable_v17
BEFORE UPDATE ON dynamic_candidate_bindings_v17
WHEN OLD.candidate_binding_id <> NEW.candidate_binding_id
  OR OLD.authorization_id <> NEW.authorization_id
  OR OLD.authorization_hmac <> NEW.authorization_hmac
  OR OLD.fetch_slot <> NEW.fetch_slot
  OR OLD.parent_reservation_id <> NEW.parent_reservation_id
  OR OLD.parent_reservation_hmac <> NEW.parent_reservation_hmac
  OR OLD.discovery_provider_hmac <> NEW.discovery_provider_hmac
  OR OLD.normalized_url_hmac <> NEW.normalized_url_hmac
  OR OLD.source_identity_hmac <> NEW.source_identity_hmac
  OR OLD.candidate_capability_hmac <> NEW.candidate_capability_hmac
  OR OLD.retry_relationship_hmac <> NEW.retry_relationship_hmac
  OR OLD.result_ordinal <> NEW.result_ordinal
  OR OLD.binding_hmac <> NEW.binding_hmac
  OR OLD.created_at_ms <> NEW.created_at_ms
  OR OLD.expires_at_ms <> NEW.expires_at_ms
  OR OLD.schema_version <> NEW.schema_version
  OR OLD.hmac_key_version <> NEW.hmac_key_version
  OR NEW.state NOT IN ('spent', 'invalidated')
  OR OLD.state NOT IN ('active', 'spent')
BEGIN
    SELECT RAISE(ABORT, 'dynamic candidate binding fields are immutable');
END;

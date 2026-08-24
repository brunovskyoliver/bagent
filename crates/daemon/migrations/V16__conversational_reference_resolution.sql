-- V16: encrypted conversational-reference ledger and protected capabilities.
-- Additive and forward-only. No historical rows are read or backfilled.

CREATE TABLE reference_session_sequences (
    scope_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL,
    origin TEXT NOT NULL CHECK (origin IN ('chat', 'automation')),
    chat_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    automation_id TEXT REFERENCES automations(id) ON DELETE CASCADE,
    next_seq INTEGER NOT NULL CHECK (next_seq > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms),
    UNIQUE (origin, session_id),
    CHECK (
        (origin = 'chat' AND chat_session_id = session_id AND automation_id IS NULL)
        OR
        (origin = 'automation' AND chat_session_id IS NULL AND automation_id IS NOT NULL)
    )
);

CREATE TABLE reference_turns (
    turn_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(turn_id) = 36 AND lower(turn_id) = turn_id
            AND turn_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(turn_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(turn_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(turn_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(turn_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(turn_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(turn_id, 9, 1) = '-'
            AND substr(turn_id, 14, 1) = '-'
            AND substr(turn_id, 19, 1) = '-'
            AND substr(turn_id, 24, 1) = '-'
        ),
    session_id TEXT NOT NULL,
    chat_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    automation_run_id TEXT REFERENCES automation_runs(id) ON DELETE CASCADE,
    session_seq INTEGER NOT NULL CHECK (session_seq > 0),
    origin TEXT NOT NULL CHECK (origin IN ('chat', 'automation')),
    state TEXT NOT NULL CHECK (state IN ('open', 'completed')),
    input_hmac BLOB NOT NULL CHECK (length(input_hmac) = 32),
    completion_code TEXT,
    producer_class TEXT,
    artifact_hmac BLOB CHECK (artifact_hmac IS NULL OR length(artifact_hmac) = 32),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    grammar_version INTEGER NOT NULL DEFAULT 1 CHECK (grammar_version > 0),
    normalization_version INTEGER NOT NULL DEFAULT 1 CHECK (normalization_version > 0),
    compatibility_epoch INTEGER NOT NULL DEFAULT 1 CHECK (compatibility_epoch > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    completed_at_ms INTEGER CHECK (completed_at_ms IS NULL OR completed_at_ms >= created_at_ms),
    open_expires_at_ms INTEGER NOT NULL CHECK (open_expires_at_ms = created_at_ms + 3600000),
    UNIQUE (session_id, session_seq),
    CHECK (
        (origin = 'chat' AND chat_session_id = session_id AND automation_run_id IS NULL)
        OR
        (origin = 'automation' AND chat_session_id IS NULL AND automation_run_id IS NOT NULL)
    ),
    CHECK (
        (state = 'open'
            AND completion_code IS NULL
            AND producer_class IS NULL
            AND artifact_hmac IS NULL
            AND completed_at_ms IS NULL)
        OR
        (state = 'completed'
            AND completion_code IN (
                'completed', 'partial', 'reference_blocked'
            )
            AND producer_class IN (
                'resolver_user_input', 'canonical_web', 'typed_mail',
                'typed_attachment', 'accepted_polish', 'legacy_assistant',
                'no_mentions', 'reference_blocked'
            )
            AND artifact_hmac IS NOT NULL
            AND completed_at_ms IS NOT NULL)
    )
);

CREATE INDEX reference_turns_session_state_seq
    ON reference_turns(session_id, state, session_seq DESC);
CREATE INDEX reference_turns_open_expiry
    ON reference_turns(state, open_expires_at_ms);
CREATE INDEX reference_turns_created
    ON reference_turns(created_at_ms);

CREATE TABLE reference_turn_staging (
    turn_id TEXT PRIMARY KEY
        REFERENCES reference_turns(turn_id) ON DELETE CASCADE,
    staged_mentions_ciphertext BLOB NOT NULL CHECK (length(staged_mentions_ciphertext) > 0),
    staged_mentions_hmac BLOB NOT NULL CHECK (length(staged_mentions_hmac) = 32),
    descriptor_version INTEGER NOT NULL CHECK (descriptor_version > 0),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    grammar_version INTEGER NOT NULL DEFAULT 1 CHECK (grammar_version > 0),
    normalization_version INTEGER NOT NULL DEFAULT 1 CHECK (normalization_version > 0),
    compatibility_epoch INTEGER NOT NULL DEFAULT 1 CHECK (compatibility_epoch > 0),
    encryption_key_version INTEGER NOT NULL CHECK (encryption_key_version > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0)
);

CREATE TABLE conversation_mentions (
    mention_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(mention_id) = 36 AND lower(mention_id) = mention_id
            AND mention_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(mention_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(mention_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(mention_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(mention_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(mention_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(mention_id, 9, 1) = '-'
            AND substr(mention_id, 14, 1) = '-'
            AND substr(mention_id, 19, 1) = '-'
            AND substr(mention_id, 24, 1) = '-'
        ),
    referent_id TEXT NOT NULL,
    turn_id TEXT NOT NULL REFERENCES reference_turns(turn_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    canonical_parent_mention_id TEXT
        REFERENCES conversation_mentions(mention_id) ON DELETE CASCADE,
    entity_kind TEXT NOT NULL CHECK (entity_kind IN (
        'person', 'organization', 'place', 'product', 'technical_standard',
        'document_title', 'public_url', 'unknown'
    )),
    text_kind TEXT NOT NULL CHECK (text_kind IN ('public_visible', 'restricted', 'opaque')),
    provenance TEXT NOT NULL CHECK (provenance IN (
        'user_authored', 'assistant_authored', 'mail_evidence',
        'web_evidence', 'attachment_evidence', 'unknown'
    )),
    assistant_lineage TEXT CHECK (assistant_lineage IN (
        'canonical', 'accepted_polish', 'legacy', 'unknown'
    )),
    producer TEXT NOT NULL CHECK (producer IN (
        'resolver_user_input', 'canonical_web', 'typed_mail',
        'typed_attachment', 'accepted_polish', 'legacy_assistant'
    )),
    visibility TEXT NOT NULL CHECK (visibility IN (
        'provider_safe', 'local_only', 'confirmation_only', 'unknown'
    )),
    sensitivity TEXT NOT NULL CHECK (sensitivity IN ('public', 'private', 'sensitive', 'unknown')),
    direct_user INTEGER NOT NULL CHECK (direct_user IN (0, 1)),
    untrusted_evidence INTEGER NOT NULL CHECK (untrusted_evidence IN (0, 1)),
    origin_ref_hmac BLOB CHECK (origin_ref_hmac IS NULL OR length(origin_ref_hmac) = 32),
    mail_body_origin TEXT CHECK (mail_body_origin IN ('local_emlx', 'mail_automation', 'unavailable')),
    public_display_ciphertext BLOB,
    public_normalized_ciphertext BLOB,
    normalized_term_hmac BLOB CHECK (
        normalized_term_hmac IS NULL OR length(normalized_term_hmac) = 32
    ),
    restricted_span_hmac BLOB CHECK (
        restricted_span_hmac IS NULL OR length(restricted_span_hmac) = 32
    ),
    safe_projection_ciphertext BLOB,
    safe_projection_hmac BLOB CHECK (
        safe_projection_hmac IS NULL OR length(safe_projection_hmac) = 32
    ),
    opaque_fingerprint BLOB CHECK (
        opaque_fingerprint IS NULL OR length(opaque_fingerprint) = 32
    ),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms = created_at_ms + 1800000),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    grammar_version INTEGER NOT NULL DEFAULT 1 CHECK (grammar_version > 0),
    normalization_version INTEGER NOT NULL DEFAULT 1 CHECK (normalization_version > 0),
    compatibility_epoch INTEGER NOT NULL DEFAULT 1 CHECK (compatibility_epoch > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    encryption_key_version INTEGER,
    CHECK (
        (text_kind = 'public_visible'
            AND public_display_ciphertext IS NOT NULL
            AND public_normalized_ciphertext IS NOT NULL
            AND normalized_term_hmac IS NOT NULL
            AND restricted_span_hmac IS NULL
            AND safe_projection_ciphertext IS NULL
            AND safe_projection_hmac IS NULL
            AND opaque_fingerprint IS NULL
            AND encryption_key_version > 0
            AND visibility = 'provider_safe'
            AND sensitivity = 'public')
        OR
        (text_kind = 'restricted'
            AND restricted_span_hmac IS NOT NULL
            AND public_display_ciphertext IS NULL
            AND public_normalized_ciphertext IS NULL
            AND normalized_term_hmac IS NULL
            AND opaque_fingerprint IS NULL
            AND (
                (safe_projection_ciphertext IS NULL AND safe_projection_hmac IS NULL)
                OR
                (safe_projection_ciphertext IS NOT NULL
                    AND safe_projection_hmac IS NOT NULL
                    AND sensitivity = 'public'
                    AND visibility = 'confirmation_only'
                    AND encryption_key_version > 0)
            )
            AND (visibility <> 'provider_safe' OR sensitivity <> 'public'))
        OR
        (text_kind = 'opaque'
            AND opaque_fingerprint IS NOT NULL
            AND public_display_ciphertext IS NULL
            AND public_normalized_ciphertext IS NULL
            AND normalized_term_hmac IS NULL
            AND restricted_span_hmac IS NULL
            AND safe_projection_ciphertext IS NULL
            AND safe_projection_hmac IS NULL
            AND (visibility <> 'provider_safe'))
    ),
    CHECK (
        (provenance = 'user_authored'
            AND producer = 'resolver_user_input'
            AND direct_user = 1
            AND untrusted_evidence = 0
            AND assistant_lineage IS NULL
            AND origin_ref_hmac IS NULL)
        OR
        (provenance = 'assistant_authored'
            AND assistant_lineage IS NOT NULL
            AND direct_user = 0
            AND visibility <> 'provider_safe')
        OR
        (provenance = 'mail_evidence'
            AND producer = 'typed_mail'
            AND mail_body_origin IS NOT NULL
            AND origin_ref_hmac IS NOT NULL
            AND direct_user = 0
            AND text_kind IN ('restricted', 'opaque'))
        OR
        (provenance = 'attachment_evidence'
            AND producer = 'typed_attachment'
            AND origin_ref_hmac IS NOT NULL
            AND direct_user = 0
            AND text_kind IN ('restricted', 'opaque'))
        OR
        (provenance = 'web_evidence'
            AND producer IN ('canonical_web', 'accepted_polish')
            AND direct_user = 0
            AND text_kind IN ('public_visible', 'restricted', 'opaque'))
        OR
        (provenance = 'unknown'
            AND direct_user = 0
            AND untrusted_evidence = 1
            AND text_kind IN ('restricted', 'opaque')
            AND visibility <> 'provider_safe')
    )
);

CREATE INDEX conversation_mentions_session_eligibility
    ON conversation_mentions(session_id, entity_kind, expires_at_ms, turn_id);
CREATE INDEX conversation_mentions_referent
    ON conversation_mentions(referent_id);
CREATE INDEX conversation_mentions_expiry
    ON conversation_mentions(expires_at_ms);
CREATE INDEX conversation_mentions_parent
    ON conversation_mentions(canonical_parent_mention_id);

CREATE TRIGGER conversation_mentions_immutable
BEFORE UPDATE ON conversation_mentions
BEGIN
    SELECT RAISE(ABORT, 'conversation mention is immutable');
END;

CREATE TRIGGER conversation_mentions_parent_binding
AFTER INSERT ON conversation_mentions
WHEN NEW.canonical_parent_mention_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM conversation_mentions parent
        WHERE parent.mention_id = NEW.canonical_parent_mention_id
          AND parent.session_id = NEW.session_id
          AND parent.referent_id = NEW.referent_id
          AND parent.entity_kind = NEW.entity_kind
          AND parent.provenance = 'web_evidence'
          AND parent.producer = 'canonical_web'
    ) THEN RAISE(ABORT, 'canonical parent binding is invalid') END;
END;

CREATE TABLE mention_derivations (
    derivation_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(derivation_id) = 36 AND lower(derivation_id) = derivation_id
            AND derivation_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(derivation_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(derivation_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(derivation_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(derivation_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(derivation_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(derivation_id, 9, 1) = '-'
            AND substr(derivation_id, 14, 1) = '-'
            AND substr(derivation_id, 19, 1) = '-'
            AND substr(derivation_id, 24, 1) = '-'
        ),
    derived_mention_id TEXT NOT NULL
        REFERENCES conversation_mentions(mention_id) ON DELETE CASCADE,
    parent_mention_id TEXT NOT NULL
        REFERENCES conversation_mentions(mention_id) ON DELETE CASCADE,
    derivation_kind TEXT NOT NULL CHECK (derivation_kind IN (
        'canonical_render_of', 'accepted_polish_of',
        'exact_structured_repeat_of', 'safe_projection_of'
    )),
    parent_ordinal INTEGER NOT NULL CHECK (parent_ordinal >= 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    UNIQUE (derived_mention_id, parent_mention_id, derivation_kind),
    CHECK (derived_mention_id <> parent_mention_id)
);

CREATE INDEX mention_derivations_parent
    ON mention_derivations(parent_mention_id);
CREATE INDEX mention_derivations_derived
    ON mention_derivations(derived_mention_id);

CREATE TRIGGER mention_derivations_validate
BEFORE INSERT ON mention_derivations
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM conversation_mentions child
        JOIN conversation_mentions parent
          ON parent.mention_id = NEW.parent_mention_id
        WHERE child.mention_id = NEW.derived_mention_id
          AND child.session_id = parent.session_id
          AND child.referent_id = parent.referent_id
    ) THEN RAISE(ABORT, 'derivation parent is outside the session or referent') END;
    SELECT CASE WHEN EXISTS (
        WITH RECURSIVE ancestors(mention_id) AS (
            SELECT parent_mention_id
            FROM mention_derivations
            WHERE derived_mention_id = NEW.parent_mention_id
            UNION ALL
            SELECT derivation.parent_mention_id
            FROM mention_derivations derivation
            JOIN ancestors
              ON derivation.derived_mention_id = ancestors.mention_id
        )
        SELECT 1 FROM ancestors WHERE mention_id = NEW.derived_mention_id
    ) THEN RAISE(ABORT, 'derivation cycle') END;
    SELECT CASE WHEN NEW.derivation_kind = 'accepted_polish_of'
        AND NOT EXISTS (
            SELECT 1
            FROM conversation_mentions parent
            WHERE parent.mention_id = NEW.parent_mention_id
              AND parent.provenance = 'web_evidence'
              AND parent.producer = 'canonical_web'
        )
        THEN RAISE(ABORT, 'accepted polish must derive from canonical web') END;
    SELECT CASE WHEN NEW.derivation_kind = 'accepted_polish_of'
        AND NOT EXISTS (
            SELECT 1
            FROM conversation_mentions child
            JOIN conversation_mentions parent
              ON parent.mention_id = NEW.parent_mention_id
            WHERE child.mention_id = NEW.derived_mention_id
              AND child.session_id = parent.session_id
              AND child.turn_id = parent.turn_id
              AND child.referent_id = parent.referent_id
              AND child.entity_kind = parent.entity_kind
              AND child.text_kind = parent.text_kind
              AND child.visibility = parent.visibility
              AND child.sensitivity = parent.sensitivity
              AND child.provenance = parent.provenance
              AND child.producer = 'accepted_polish'
              AND COALESCE(child.normalized_term_hmac, child.restricted_span_hmac,
                           child.opaque_fingerprint) =
                  COALESCE(parent.normalized_term_hmac, parent.restricted_span_hmac,
                           parent.opaque_fingerprint)
              AND COALESCE(child.origin_ref_hmac, zeroblob(32)) =
                  COALESCE(parent.origin_ref_hmac, zeroblob(32))
        )
        THEN RAISE(ABORT, 'accepted polish changes protected provenance') END;
END;

CREATE TRIGGER mention_derivations_immutable
BEFORE UPDATE ON mention_derivations
BEGIN
    SELECT RAISE(ABORT, 'mention derivation is immutable');
END;

CREATE TABLE mention_anchors (
    anchor_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(anchor_id) = 36 AND lower(anchor_id) = anchor_id
            AND anchor_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(anchor_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(anchor_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(anchor_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(anchor_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(anchor_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(anchor_id, 9, 1) = '-'
            AND substr(anchor_id, 14, 1) = '-'
            AND substr(anchor_id, 19, 1) = '-'
            AND substr(anchor_id, 24, 1) = '-'
        ),
    mention_id TEXT NOT NULL REFERENCES conversation_mentions(mention_id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL REFERENCES reference_turns(turn_id) ON DELETE CASCADE,
    anchor_kind TEXT NOT NULL CHECK (anchor_kind IN ('visible', 'opaque')),
    display_class TEXT NOT NULL CHECK (display_class IN (
        'user_input', 'canonical', 'accepted_polish', 'restricted_output', 'opaque'
    )),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    start_utf8 INTEGER CHECK (start_utf8 IS NULL OR start_utf8 >= 0),
    end_utf8 INTEGER CHECK (end_utf8 IS NULL OR end_utf8 >= 0),
    visible_span_hmac BLOB CHECK (
        visible_span_hmac IS NULL OR length(visible_span_hmac) = 32
    ),
    opaque_anchor_hmac BLOB CHECK (
        opaque_anchor_hmac IS NULL OR length(opaque_anchor_hmac) = 32
    ),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    UNIQUE (turn_id, display_class, ordinal),
    CHECK (
        (anchor_kind = 'visible'
            AND start_utf8 IS NOT NULL
            AND end_utf8 IS NOT NULL
            AND end_utf8 >= start_utf8
            AND visible_span_hmac IS NOT NULL
            AND opaque_anchor_hmac IS NULL)
        OR
        (anchor_kind = 'opaque'
            AND start_utf8 IS NULL
            AND end_utf8 IS NULL
            AND visible_span_hmac IS NULL
            AND opaque_anchor_hmac IS NOT NULL
            AND display_class = 'opaque')
    )
);

CREATE INDEX mention_anchors_mention ON mention_anchors(mention_id);
CREATE INDEX mention_anchors_turn_ordinal ON mention_anchors(turn_id, ordinal);

CREATE TRIGGER mention_anchors_validate
BEFORE INSERT ON mention_anchors
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM conversation_mentions mention
        WHERE mention.mention_id = NEW.mention_id
          AND mention.turn_id = NEW.turn_id
    ) THEN RAISE(ABORT, 'anchor mention binding is invalid') END;
END;

CREATE TRIGGER mention_anchors_immutable
BEFORE UPDATE ON mention_anchors
BEGIN
    SELECT RAISE(ABORT, 'mention anchor is immutable');
END;

CREATE TABLE mention_web_mappings (
    mapping_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(mapping_id) = 36 AND lower(mapping_id) = mapping_id
            AND mapping_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(mapping_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(mapping_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(mapping_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(mapping_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(mapping_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(mapping_id, 9, 1) = '-'
            AND substr(mapping_id, 14, 1) = '-'
            AND substr(mapping_id, 19, 1) = '-'
            AND substr(mapping_id, 24, 1) = '-'
        ),
    mention_id TEXT NOT NULL REFERENCES conversation_mentions(mention_id) ON DELETE CASCADE,
    canonical_anchor_id TEXT NOT NULL REFERENCES mention_anchors(anchor_id) ON DELETE CASCADE,
    source_ordinal INTEGER NOT NULL CHECK (source_ordinal >= 0),
    evidence_id_hmac BLOB NOT NULL CHECK (length(evidence_id_hmac) = 32),
    source_identity_ciphertext BLOB NOT NULL CHECK (length(source_identity_ciphertext) > 0),
    source_identity_hmac BLOB NOT NULL CHECK (length(source_identity_hmac) = 32),
    public_url_ciphertext BLOB NOT NULL CHECK (length(public_url_ciphertext) > 0),
    public_url_hmac BLOB NOT NULL CHECK (length(public_url_hmac) = 32),
    authority TEXT NOT NULL CHECK (authority IN (
        'first_party', 'authoritative_reference', 'other'
    )),
    network_policy_version INTEGER NOT NULL CHECK (network_policy_version > 0),
    validated_at_ms INTEGER NOT NULL CHECK (validated_at_ms >= 0),
    encryption_key_version INTEGER NOT NULL CHECK (encryption_key_version > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    UNIQUE (mention_id, source_ordinal),
    UNIQUE (mention_id, evidence_id_hmac, public_url_hmac)
);

CREATE INDEX mention_web_mappings_mention ON mention_web_mappings(mention_id);
CREATE INDEX mention_web_mappings_evidence ON mention_web_mappings(evidence_id_hmac);
CREATE INDEX mention_web_mappings_source ON mention_web_mappings(source_identity_hmac);
CREATE INDEX mention_web_mappings_url ON mention_web_mappings(public_url_hmac);

CREATE TRIGGER mention_web_mappings_validate
BEFORE INSERT ON mention_web_mappings
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM conversation_mentions mention
        JOIN mention_anchors anchor
          ON anchor.anchor_id = NEW.canonical_anchor_id
         AND anchor.mention_id = NEW.mention_id
         AND anchor.display_class = 'canonical'
        WHERE mention.mention_id = NEW.mention_id
          AND mention.provenance = 'web_evidence'
          AND mention.producer = 'canonical_web'
          AND mention.text_kind = 'public_visible'
          AND mention.visibility = 'provider_safe'
          AND mention.sensitivity = 'public'
          AND NOT EXISTS (
              SELECT 1 FROM mention_derivations derivation
              WHERE derivation.derived_mention_id = mention.mention_id
          )
    ) THEN RAISE(ABORT, 'canonical web mapping is invalid') END;
END;

CREATE TRIGGER mention_web_mappings_immutable
BEFORE UPDATE ON mention_web_mappings
BEGIN
    SELECT RAISE(ABORT, 'web mapping is immutable');
END;

CREATE TABLE reference_confirmations (
    confirmation_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(confirmation_id) = 36 AND lower(confirmation_id) = confirmation_id
            AND confirmation_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(confirmation_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 9, 1) = '-'
            AND substr(confirmation_id, 14, 1) = '-'
            AND substr(confirmation_id, 19, 1) = '-'
            AND substr(confirmation_id, 24, 1) = '-'
        ),
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    initiating_turn_id TEXT NOT NULL REFERENCES reference_turns(turn_id) ON DELETE CASCADE,
    mention_id TEXT REFERENCES conversation_mentions(mention_id) ON DELETE RESTRICT,
    referent_id TEXT NOT NULL,
    provider_scope TEXT NOT NULL CHECK (provider_scope IN (
        'web_search_fetch', 'direct_public_fetch'
    )),
    sensitivity TEXT NOT NULL CHECK (sensitivity IN ('public', 'private', 'sensitive')),
    proposal_ciphertext BLOB NOT NULL CHECK (length(proposal_ciphertext) > 0),
    normalized_term_hmac BLOB NOT NULL CHECK (length(normalized_term_hmac) = 32),
    normalization_version INTEGER NOT NULL CHECK (normalization_version > 0),
    compatibility_epoch INTEGER NOT NULL CHECK (compatibility_epoch > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms = created_at_ms + 300000),
    encryption_key_version INTEGER NOT NULL CHECK (encryption_key_version > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0)
);

CREATE INDEX reference_confirmations_session_expiry
    ON reference_confirmations(session_id, expires_at_ms);
CREATE INDEX reference_confirmations_referent
    ON reference_confirmations(referent_id);

CREATE TABLE reference_confirmation_tombstones (
    confirmation_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(confirmation_id) = 36 AND lower(confirmation_id) = confirmation_id
            AND confirmation_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(confirmation_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(confirmation_id, 9, 1) = '-'
            AND substr(confirmation_id, 14, 1) = '-'
            AND substr(confirmation_id, 19, 1) = '-'
            AND substr(confirmation_id, 24, 1) = '-'
        ),
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    initiating_turn_id TEXT NOT NULL,
    execution_turn_id TEXT,
    referent_id TEXT NOT NULL,
    provider_scope TEXT NOT NULL CHECK (provider_scope IN (
        'web_search_fetch', 'direct_public_fetch'
    )),
    normalized_term_hmac BLOB NOT NULL CHECK (length(normalized_term_hmac) = 32),
    terminal_state TEXT NOT NULL CHECK (terminal_state IN (
        'consumed', 'expired', 'edited', 'term_mismatch', 'invalidated'
    )),
    compatibility_epoch INTEGER NOT NULL CHECK (compatibility_epoch > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    terminal_at_ms INTEGER NOT NULL CHECK (terminal_at_ms >= created_at_ms),
    delete_after_ms INTEGER NOT NULL CHECK (delete_after_ms = terminal_at_ms + 86400000),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0)
);

CREATE INDEX reference_confirmation_tombstones_retention
    ON reference_confirmation_tombstones(delete_after_ms);

CREATE TABLE query_authorizations (
    authorization_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(authorization_id) = 36 AND lower(authorization_id) = authorization_id
            AND authorization_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(authorization_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 9, 1) = '-'
            AND substr(authorization_id, 14, 1) = '-'
            AND substr(authorization_id, 19, 1) = '-'
            AND substr(authorization_id, 24, 1) = '-'
        ),
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    initiating_turn_id TEXT NOT NULL REFERENCES reference_turns(turn_id) ON DELETE CASCADE,
    execution_turn_id TEXT NOT NULL REFERENCES reference_turns(turn_id) ON DELETE CASCADE,
    referent_id TEXT NOT NULL,
    mention_id TEXT REFERENCES conversation_mentions(mention_id) ON DELETE RESTRICT,
    confirmation_id TEXT REFERENCES reference_confirmation_tombstones(confirmation_id) ON DELETE RESTRICT,
    authorization_method TEXT NOT NULL CHECK (authorization_method IN (
        'current_user', 'canonical_web', 'confirmed'
    )),
    provider_scope TEXT NOT NULL CHECK (provider_scope IN (
        'web_search_fetch', 'direct_public_fetch'
    )),
    query_plan_hmac BLOB NOT NULL CHECK (length(query_plan_hmac) = 32),
    permit_nonce_hmac BLOB NOT NULL CHECK (length(permit_nonce_hmac) = 32),
    plan_version INTEGER NOT NULL CHECK (plan_version > 0),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    grammar_version INTEGER NOT NULL DEFAULT 1 CHECK (grammar_version > 0),
    normalization_version INTEGER NOT NULL DEFAULT 1 CHECK (normalization_version > 0),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    compatibility_epoch INTEGER NOT NULL CHECK (compatibility_epoch > 0),
    configuration_epoch INTEGER NOT NULL CHECK (configuration_epoch > 0),
    process_epoch INTEGER NOT NULL CHECK (process_epoch > 0),
    search_budget INTEGER NOT NULL CHECK (search_budget BETWEEN 0 AND 2),
    fetch_budget INTEGER NOT NULL CHECK (fetch_budget BETWEEN 0 AND 5),
    reserved_searches INTEGER NOT NULL DEFAULT 0
        CHECK (reserved_searches BETWEEN 0 AND search_budget),
    reserved_fetches INTEGER NOT NULL DEFAULT 0
        CHECK (reserved_fetches BETWEEN 0 AND fetch_budget),
    issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms = issued_at_ms + 300000),
    CHECK (
        (authorization_method = 'confirmed' AND confirmation_id IS NOT NULL)
        OR
        (authorization_method <> 'confirmed' AND confirmation_id IS NULL)
    )
);

CREATE INDEX query_authorizations_execution_turn
    ON query_authorizations(execution_turn_id);
CREATE INDEX query_authorizations_expiry
    ON query_authorizations(expires_at_ms);
CREATE INDEX query_authorizations_nonce
    ON query_authorizations(permit_nonce_hmac);

CREATE TABLE query_authorization_operations (
    authorization_id TEXT NOT NULL
        REFERENCES query_authorizations(authorization_id) ON DELETE CASCADE,
    operation_ordinal INTEGER NOT NULL CHECK (operation_ordinal >= 0),
    operation_hmac BLOB NOT NULL CHECK (length(operation_hmac) = 32),
    operation_kind TEXT NOT NULL CHECK (operation_kind IN ('search', 'fetch')),
    provider TEXT NOT NULL CHECK (provider IN (
        'tavily', 'duckduckgo', 'wikipedia', 'direct'
    )),
    max_attempts INTEGER NOT NULL CHECK (max_attempts IN (1, 2)),
    reserved_attempts INTEGER NOT NULL DEFAULT 0
        CHECK (reserved_attempts BETWEEN 0 AND max_attempts),
    alternative_group TEXT,
    PRIMARY KEY (authorization_id, operation_ordinal),
    UNIQUE (authorization_id, operation_hmac),
    CHECK ((operation_kind = 'search') OR alternative_group IS NULL),
    CHECK (provider <> 'direct' OR operation_kind = 'fetch')
);

CREATE INDEX query_authorization_operations_hmac
    ON query_authorization_operations(operation_hmac);
CREATE UNIQUE INDEX query_authorization_operations_alternative
    ON query_authorization_operations(authorization_id, alternative_group)
    WHERE alternative_group IS NOT NULL;

CREATE TRIGGER query_authorization_operations_validate
BEFORE INSERT ON query_authorization_operations
BEGIN
    SELECT CASE WHEN NEW.alternative_group IS NOT NULL AND NEW.operation_ordinal <> 1
        THEN RAISE(ABORT, 'second search alternative must use ordinal one') END;
    SELECT CASE WHEN NEW.alternative_group IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM query_authorization_operations existing
        WHERE existing.authorization_id = NEW.authorization_id
          AND existing.operation_ordinal = 0
          AND existing.operation_kind = 'search'
    ) THEN RAISE(ABORT, 'second search alternative requires primary search') END;
END;

CREATE TABLE provider_attempt_reservations (
    reservation_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(reservation_id) = 36 AND lower(reservation_id) = reservation_id
            AND reservation_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(reservation_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(reservation_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(reservation_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(reservation_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(reservation_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(reservation_id, 9, 1) = '-'
            AND substr(reservation_id, 14, 1) = '-'
            AND substr(reservation_id, 19, 1) = '-'
            AND substr(reservation_id, 24, 1) = '-'
        ),
    authorization_id TEXT NOT NULL,
    operation_ordinal INTEGER NOT NULL CHECK (operation_ordinal >= 0),
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    reserved_at_ms INTEGER NOT NULL CHECK (reserved_at_ms >= 0),
    FOREIGN KEY (authorization_id, operation_ordinal)
        REFERENCES query_authorization_operations(authorization_id, operation_ordinal)
        ON DELETE CASCADE,
    UNIQUE (authorization_id, operation_ordinal, attempt_number)
);

CREATE INDEX provider_attempt_reservations_operation
    ON provider_attempt_reservations(authorization_id, operation_ordinal);

CREATE TABLE query_replay_tombstones (
    authorization_id TEXT PRIMARY KEY NOT NULL
        CHECK (
            length(authorization_id) = 36 AND lower(authorization_id) = authorization_id
            AND authorization_id NOT GLOB '*[^0-9a-f-]*'
            AND substr(authorization_id, 1, 8) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 10, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 15, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 20, 4) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 25, 12) NOT GLOB '*[^0-9a-f]*'
            AND substr(authorization_id, 9, 1) = '-'
            AND substr(authorization_id, 14, 1) = '-'
            AND substr(authorization_id, 19, 1) = '-'
            AND substr(authorization_id, 24, 1) = '-'
        ),
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    initiating_turn_id TEXT NOT NULL,
    execution_turn_id TEXT NOT NULL,
    referent_id TEXT NOT NULL,
    confirmation_id TEXT REFERENCES reference_confirmation_tombstones(confirmation_id)
        ON DELETE RESTRICT,
    authorization_method TEXT NOT NULL CHECK (authorization_method IN (
        'current_user', 'canonical_web', 'confirmed'
    )),
    provider_scope TEXT NOT NULL CHECK (provider_scope IN (
        'web_search_fetch', 'direct_public_fetch'
    )),
    query_plan_hmac BLOB NOT NULL CHECK (length(query_plan_hmac) = 32),
    permit_nonce_hmac BLOB NOT NULL CHECK (length(permit_nonce_hmac) = 32),
    final_reserved_searches INTEGER NOT NULL CHECK (final_reserved_searches BETWEEN 0 AND 2),
    final_reserved_fetches INTEGER NOT NULL CHECK (final_reserved_fetches BETWEEN 0 AND 5),
    hmac_key_version INTEGER NOT NULL CHECK (hmac_key_version > 0),
    compatibility_epoch INTEGER NOT NULL CHECK (compatibility_epoch > 0),
    configuration_epoch INTEGER NOT NULL CHECK (configuration_epoch > 0),
    process_epoch INTEGER NOT NULL CHECK (process_epoch > 0),
    terminal_state TEXT NOT NULL CHECK (terminal_state IN (
        'completed', 'expired', 'exhausted', 'failed_closed', 'abandoned'
    )),
    terminal_at_ms INTEGER NOT NULL CHECK (terminal_at_ms >= 0),
    delete_after_ms INTEGER NOT NULL CHECK (delete_after_ms = terminal_at_ms + 86400000)
);

CREATE INDEX query_replay_tombstones_retention
    ON query_replay_tombstones(delete_after_ms);
CREATE INDEX query_replay_tombstones_nonce
    ON query_replay_tombstones(permit_nonce_hmac);

CREATE TRIGGER reference_confirmations_immutable
BEFORE UPDATE ON reference_confirmations
BEGIN
    SELECT RAISE(ABORT, 'confirmation is immutable');
END;

CREATE TRIGGER reference_confirmation_tombstones_immutable
BEFORE UPDATE ON reference_confirmation_tombstones
BEGIN
    SELECT RAISE(ABORT, 'confirmation tombstone is immutable');
END;

CREATE TRIGGER query_authorizations_immutable
BEFORE UPDATE ON query_authorizations
WHEN OLD.authorization_id <> NEW.authorization_id
  OR OLD.session_id <> NEW.session_id
  OR OLD.initiating_turn_id <> NEW.initiating_turn_id
  OR OLD.execution_turn_id <> NEW.execution_turn_id
  OR OLD.referent_id <> NEW.referent_id
  OR COALESCE(OLD.mention_id, '') <> COALESCE(NEW.mention_id, '')
  OR COALESCE(OLD.confirmation_id, '') <> COALESCE(NEW.confirmation_id, '')
  OR OLD.authorization_method <> NEW.authorization_method
  OR OLD.provider_scope <> NEW.provider_scope
  OR OLD.query_plan_hmac <> NEW.query_plan_hmac
  OR OLD.permit_nonce_hmac <> NEW.permit_nonce_hmac
  OR OLD.plan_version <> NEW.plan_version
  OR OLD.schema_version <> NEW.schema_version
  OR OLD.grammar_version <> NEW.grammar_version
  OR OLD.normalization_version <> NEW.normalization_version
  OR OLD.hmac_key_version <> NEW.hmac_key_version
  OR OLD.compatibility_epoch <> NEW.compatibility_epoch
  OR OLD.configuration_epoch <> NEW.configuration_epoch
  OR OLD.process_epoch <> NEW.process_epoch
  OR OLD.search_budget <> NEW.search_budget
  OR OLD.fetch_budget <> NEW.fetch_budget
  OR OLD.issued_at_ms <> NEW.issued_at_ms
  OR OLD.expires_at_ms <> NEW.expires_at_ms
BEGIN
    SELECT RAISE(ABORT, 'authorization is immutable');
END;

CREATE TRIGGER query_replay_tombstones_immutable
BEFORE UPDATE ON query_replay_tombstones
BEGIN
    SELECT RAISE(ABORT, 'query tombstone is immutable');
END;

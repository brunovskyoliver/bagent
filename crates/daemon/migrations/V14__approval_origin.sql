-- V14: Pending approvals carry provenance (which automation/run asked) so the
-- approval UI can identify the originating automation. NULL for chat/REST.
ALTER TABLE pending_approvals ADD COLUMN origin_json TEXT;

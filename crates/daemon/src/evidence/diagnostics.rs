use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_TURNS: usize = 1_000;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TURN_BYTES: usize = 256 * 1024;
const MAX_EVENTS_PER_TURN: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiagnosticTrace {
    version: u16,
    turn_id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    events: Vec<Value>,
}

pub(crate) struct DiagnosticRecorder {
    directory: PathBuf,
    lock: Mutex<()>,
}

impl DiagnosticRecorder {
    pub(crate) fn new(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        let recorder = Self {
            directory,
            lock: Mutex::new(()),
        };
        recorder.prune()?;
        Ok(recorder)
    }

    pub(crate) fn record(&self, event: &Value) {
        if let Err(error) = self.record_inner(event) {
            tracing::warn!(reason = %error, "evidence diagnostic event was not persisted");
        }
    }

    pub(crate) fn export(&self, turn_id: &str) -> Result<Value> {
        let _guard = self.lock.lock().expect("diagnostic recorder lock");
        let path = self.trace_path(turn_id);
        let bytes = fs::read(path).map_err(|_| anyhow!("evidence trace not found"))?;
        let trace: DiagnosticTrace = serde_json::from_slice(&bytes)?;
        if trace.turn_id != turn_id {
            return Err(anyhow!("evidence trace correlation mismatch"));
        }
        Ok(serde_json::to_value(trace)?)
    }

    fn record_inner(&self, event: &Value) -> Result<()> {
        let Some(sanitized) = sanitize_event(event) else {
            return Ok(());
        };
        let turn_id = sanitized
            .get("turn_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing turn correlation"))?;
        let _guard = self.lock.lock().expect("diagnostic recorder lock");
        let now = Utc::now();
        let path = self.trace_path(turn_id);
        let mut trace = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<DiagnosticTrace>(&bytes).ok())
            .filter(|trace| trace.turn_id == turn_id)
            .unwrap_or_else(|| DiagnosticTrace {
                version: 1,
                turn_id: turn_id.to_string(),
                created_at: now,
                updated_at: now,
                events: Vec::new(),
            });
        trace.updated_at = now;
        trace.events.push(sanitized);
        if trace.events.len() > MAX_EVENTS_PER_TURN {
            trace
                .events
                .drain(..trace.events.len().saturating_sub(MAX_EVENTS_PER_TURN));
        }
        while serde_json::to_vec(&trace)?.len() > MAX_TURN_BYTES && trace.events.len() > 1 {
            trace.events.remove(0);
        }
        let encoded = serde_json::to_vec_pretty(&trace)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, encoded)?;
        fs::rename(temporary, path)?;
        // Enforce the global age/count/size bounds on every append. This also
        // bounds a burst of malformed turns that never reach a terminal event.
        self.prune_locked()
    }

    fn prune(&self) -> Result<()> {
        let _guard = self.lock.lock().expect("diagnostic recorder lock");
        self.prune_locked()
    }

    fn prune_locked(&self) -> Result<()> {
        let now = SystemTime::now();
        let mut files = trace_files(&self.directory)?;
        for (_, modified, path) in &files {
            if now.duration_since(*modified).unwrap_or_default() > MAX_AGE {
                let _ = fs::remove_file(path);
            }
        }
        files = trace_files(&self.directory)?;
        files.sort_by_key(|(_, modified, _)| *modified);
        while files.len() > MAX_TURNS {
            let (_, _, path) = files.remove(0);
            let _ = fs::remove_file(path);
        }
        let mut total = files.iter().map(|(size, _, _)| *size).sum::<u64>();
        while total > MAX_TOTAL_BYTES && files.len() > 1 {
            let (size, _, path) = files.remove(0);
            let _ = fs::remove_file(path);
            total = total.saturating_sub(size);
        }
        Ok(())
    }

    fn trace_path(&self, turn_id: &str) -> PathBuf {
        let digest = Sha256::digest(turn_id.as_bytes());
        let name = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.directory.join(format!("{name}.json"))
    }
}

fn trace_files(directory: &Path) -> Result<Vec<(u64, SystemTime, PathBuf)>> {
    Ok(fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            Some((
                metadata.len(),
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                path,
            ))
        })
        .collect())
}

fn sanitize_event(event: &Value) -> Option<Value> {
    let event_type = event.get("type")?.as_str()?;
    if !matches!(
        event_type,
        "evidence_phase"
            | "logical_activity_started"
            | "logical_activity_completed"
            | "evidence_validation"
            | "evidence_polish"
            | "evidence_outcome"
            | "evidence_acquisition_diagnostic"
    ) {
        return None;
    }
    let turn_id = safe_string(event.get("turn_id")?, 128)?;
    let mut sanitized = json!({
        "type": event_type,
        "turn_id": turn_id,
    });
    let allowed_strings = [
        "phase",
        "model_id",
        "activity_id",
        "normalized_operation",
        "argument_hash",
        "execution_status",
        "contribution",
        "failure_reason",
        "state",
        "kind",
        "decision",
        "status",
        "body_origin",
        "provider",
        "provider_status",
        "source_identity",
        "authority",
        "extraction_result",
        "rejection_reason",
    ];
    for key in allowed_strings {
        if let Some(value) = event.get(key).and_then(|value| safe_string(value, 160)) {
            sanitized[key] = Value::String(value);
        }
    }
    let allowed_numbers = [
        "completed",
        "total",
        "duration_ms",
        "evidence_count",
        "attempt_count",
        "retries",
        "duplicates_suppressed",
        "acquired",
        "requested",
        "source_count",
        "missing_count",
        "conflict_count",
        "exclusion_count",
        "candidate_count",
        "rank",
        "relevance_score",
        "search_attempts_used",
        "search_attempt_budget",
        "fetch_attempts_used",
        "fetch_attempt_budget",
    ];
    for key in allowed_numbers {
        if let Some(value) = event.get(key).and_then(Value::as_u64) {
            sanitized[key] = json!(value);
        }
    }
    for key in ["timed_out", "fallback", "repair", "eligible"] {
        if let Some(value) = event.get(key).and_then(Value::as_bool) {
            sanitized[key] = json!(value);
        }
    }
    if let Some(domains) = event.get("source_domains").and_then(Value::as_array) {
        sanitized["source_domains"] = Value::Array(
            domains
                .iter()
                .take(16)
                .filter_map(|value| safe_string(value, 253).map(Value::String))
                .collect(),
        );
    }
    Some(sanitized)
}

fn safe_string(value: &Value, maximum: usize) -> Option<String> {
    let value = value.as_str()?;
    if value.is_empty()
        || value.len() > maximum
        || value.chars().any(char::is_control)
        || value.contains(['\n', '\r'])
    {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn event(turn_id: &str, private: &str) -> Value {
        json!({
            "type": "logical_activity_completed",
            "turn_id": turn_id,
            "activity_id": "evidence:abc",
            "normalized_operation": "mail.read",
            "argument_hash": "hash",
            "execution_status": "succeeded",
            "contribution": "satisfied",
            "evidence_count": 1,
            "source_domains": [],
            "duration_ms": 12,
            "attempt_count": 1,
            "retries": 0,
            "duplicates_suppressed": 0,
            "failure_reason": null,
            "body_origin": "mail_automation",
            "prompt": private,
            "subject": private,
            "body": private,
            "raw_arguments": {"rowid": private},
            "answer": private,
            "reasoning_content": private,
            "token": private,
        })
    }

    #[test]
    fn export_is_structural_and_forbidden_fixture_values_are_absent() {
        let directory = TempDir::new().unwrap();
        let recorder = DiagnosticRecorder::new(directory.path().to_path_buf()).unwrap();
        let private = "PRIVATE_FIXTURE_sender_subject_body_prompt_token";
        recorder.record(&event("turn-private", private));
        recorder.record(&json!({
            "type": "evidence_validation",
            "turn_id": "turn-private",
            "decision": "bundle_partial",
            "eligible": true,
            "missing_count": 1,
            "conflict_count": 0,
            "exclusion_count": 2,
            "raw_decision_context": private,
        }));
        recorder.record(&json!({
            "type": "evidence_polish",
            "turn_id": "turn-private",
            "status": "rejected",
            "model_output": private,
            "validation_detail": private,
        }));
        let exported = recorder.export("turn-private").unwrap().to_string();
        assert!(exported.contains("\"normalized_operation\":\"mail.read\""));
        assert!(exported.contains("\"decision\":\"bundle_partial\""));
        assert!(exported.contains("\"status\":\"rejected\""));
        assert!(exported.contains("\"body_origin\":\"mail_automation\""));
        assert!(!exported.contains(private));
        for forbidden in [
            "prompt",
            "subject",
            "body",
            "raw_arguments",
            "answer",
            "reasoning_content",
            "token",
        ] {
            assert!(!exported.contains(&format!("\"{forbidden}\"")));
        }
    }

    #[test]
    fn per_turn_size_rotation_is_bounded() {
        let directory = TempDir::new().unwrap();
        let recorder = DiagnosticRecorder::new(directory.path().to_path_buf()).unwrap();
        for index in 0..700 {
            let mut value = event("turn-volume", "private");
            value["activity_id"] = json!(format!("evidence:{index:0150}"));
            recorder.record(&value);
        }
        let export = recorder.export("turn-volume").unwrap();
        assert!(serde_json::to_vec(&export).unwrap().len() <= MAX_TURN_BYTES);
        assert!(export["events"].as_array().unwrap().len() <= MAX_EVENTS_PER_TURN);
    }

    #[test]
    fn retention_prunes_by_age_and_count() {
        let directory = TempDir::new().unwrap();
        let recorder = DiagnosticRecorder::new(directory.path().to_path_buf()).unwrap();
        for index in 0..1_010 {
            recorder.record(&event(&format!("turn-{index}"), "private"));
        }
        recorder.prune().unwrap();
        assert!(trace_files(directory.path()).unwrap().len() <= MAX_TURNS);

        let old_path = directory.path().join("old.json");
        fs::write(
            &old_path,
            serde_json::to_vec(&DiagnosticTrace {
                version: 1,
                turn_id: "old".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                events: vec![],
            })
            .unwrap(),
        )
        .unwrap();
        let old = filetime::FileTime::from_system_time(
            SystemTime::now() - MAX_AGE - Duration::from_secs(1),
        );
        filetime::set_file_mtime(&old_path, old).unwrap();
        recorder.prune().unwrap();
        assert!(!old_path.exists());
    }

    #[test]
    fn every_append_enforces_global_size_for_nonterminal_turns() {
        let directory = TempDir::new().unwrap();
        let recorder = DiagnosticRecorder::new(directory.path().to_path_buf()).unwrap();
        for index in 0..16 {
            fs::write(
                directory.path().join(format!("burst-{index}.json")),
                vec![b'x'; 600 * 1024],
            )
            .unwrap();
        }

        recorder.record(&event("still-running", "private"));

        let total = trace_files(directory.path())
            .unwrap()
            .iter()
            .map(|(size, _, _)| size)
            .sum::<u64>();
        assert!(total <= MAX_TOTAL_BYTES);
    }
}

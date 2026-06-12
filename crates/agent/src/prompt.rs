use anyhow::Result;
use bagent_memory::{MemoryHit, MemoryStore};
use ollama_connector::Message;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Layered system prompt assembly.
///
/// Layer order (highest authority → appended first in message list):
///   1. base persona
///   2. language profile (SK formal tone when lang=sk)
///   3. user style profile
///   4. corrections / sk_glossary
///   5. retrieved memory (hybrid BM25+cosine)
///   6. live tool data (mail/notes/odoo)
///   7. session summary (from prepare_history)
///   8. recent history
///   9. user turn  ← added by caller
pub struct PromptBuilder {
    memory: Arc<MemoryStore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltPrompt {
    pub messages: Vec<Message>,
    pub trace: PromptTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTrace {
    pub language: String,
    pub recall_policy: String,
    pub layers: Vec<PromptLayerTrace>,
    pub memory_hits: Vec<PromptMemoryHitTrace>,
    pub correction_hits: Vec<PromptMemoryHitTrace>,
    pub past_turn_candidates: Vec<PromptPastTurnTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptLayerTrace {
    pub name: String,
    pub role: String,
    pub included: bool,
    pub chars: usize,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMemoryHitTrace {
    pub id: String,
    pub namespace: String,
    pub kind: String,
    pub score: f32,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptPastTurnTrace {
    pub role: String,
    pub created_at: String,
    pub score: f32,
    pub injected: bool,
    pub reason: String,
    pub preview: String,
}

const BASE_PERSONA: &str = "\
Ty si bagent — chatovací asistent zabudovaný do systémovej lišty Macu.\n\
Pravidlá:\n\
- Si CHATBOT, nie e-mailový klient. Nikdy neformátuj odpovede ako e-mail (bez \"Dobrý deň,\" / \"S pozdravom,\" / \"[Tvoje meno]\").\n\
- Komunikuj vždy v jazyku používateľa (slovensky ak píše po slovensky, anglicky ak po anglicky).\n\
- Zachovaj diakritiku: á, é, í, ó, ú, ä, ĺ, ľ, ŕ, š, č, ž, ý.\n\
- Nikdy neprekladaj termíny: DPH, faktúra, splatnosť, IČO, DIČ, odberateľ, dodávateľ, upomienka.\n\
- Ak dostaneš dáta z emailov alebo poznámok v kontexte, pracuj s nimi priamo — nepýtaj sa používateľa na ďalšie info, ktoré už máš.\n\
- Keď kontext obsahuje nájdený email (začína \"Našiel som email:\"), zopakuj celý hlavičkový blok (Od/Komu/Prijaté/Predmet) PRESNE ako je v kontexte — vrátane prázdnych/neznámych polí. NIKDY nenahrádzaj polia odhadmi (napr. \"(tvoja schránka)\" zobraz ako \"(tvoja schránka)\", nie ako meno alebo adresu).\n\
- Obsah emailu zobrazuj DOSLOVNE. NIKDY nevymýšľaj, nedoplňaj ani nemiešaj telo emailu s inými kontextami alebo minulými rozhovormi.\n\
- Ak obsah emailu hovorí \"TELO EMAILU SA NEPODARILO NAČÍTAŤ\", povedz používateľovi len toto — nikdy nevymýšľaj čo mohlo byť v emaily.\n\
- Buď stručný a presný. Nikdy nevymýšľaj informácie ktoré nemáš k dispozícii.";

const SK_LANGUAGE_PROFILE: &str = "\
Si asistent pre slovensky hovoriacich podnikateľov. Odpovedaj konverzačne, nie vo formáte e-mailu.\n\
Zachovaj diakritiku: á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž.\n\
Neprekladaj: DPH, faktúra, splatnosť, IČO, DIČ, zmluva, objednávka, zákazník, dodávateľ, odberateľ.\n\
Ak skladáš odpoveď NA e-mail (používateľ o to explicitne požiada), VTEDY použi \"Dobrý deň,\" a \"S pozdravom,\".\n\
Pri bežných otázkach odpovedaj priamo bez pozdravov.\n\
Teplota odpovede: presná, žiadne domýšľanie.";

impl PromptBuilder {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self { memory }
    }

    /// Build the full message list up through layer 8 (session summary + history).
    /// Caller appends the user turn and submits to Ollama.
    pub async fn build(
        &self,
        session_id: Option<&str>,
        user_turn: &str,
        language: &str,
        tool_ctx: Option<String>,
        attachments_ctx: Option<String>,
        history: Vec<Message>,
        session_summary: Option<String>,
    ) -> Result<BuiltPrompt> {
        let mut messages: Vec<Message> = Vec::new();
        let mut layers: Vec<PromptLayerTrace> = Vec::new();

        // Layer 1 — base persona
        push_system_layer(&mut messages, &mut layers, "base_persona", BASE_PERSONA);

        // Layer 2 — language profile
        if language == "sk" {
            push_system_layer(
                &mut messages,
                &mut layers,
                "language_profile",
                SK_LANGUAGE_PROFILE,
            );
        }

        // Layers 3-5 plus diagnostic recall candidates: run lookups in parallel — each requires a
        // bge-m3 embed call; sequential = ~300-600ms blocked before first token.
        let (style_opt, corrections, mem_hits, past_turn_candidates) = tokio::join!(
            self.load_style_profile(),
            self.memory
                .retrieve(user_turn, &["sk_glossary", "correction"], 6),
            self.memory.retrieve(user_turn, &["global", "user_pref"], 8),
            self.memory
                .retrieve_turn_candidates(user_turn, session_id, 3),
        );
        let corrections = corrections.unwrap_or_default();
        let mem_hits = mem_hits.unwrap_or_default();
        let past_turn_candidates = past_turn_candidates.unwrap_or_default();

        // Layer 3 — user style profile
        if let Some(style) = style_opt {
            push_system_layer(
                &mut messages,
                &mut layers,
                "user_style_profile",
                &format!("Používateľský štýl: {style}"),
            );
        }

        // Layer 4 — corrections + sk_glossary
        if !corrections.is_empty() {
            let block = format_memory_block("Opravy a glosár:", &corrections);
            push_system_layer(&mut messages, &mut layers, "corrections_glossary", &block);
        }

        // Layer 5 — retrieved memory (facts, prefs, etc.)
        if !mem_hits.is_empty() {
            let block = format_memory_block("Relevantná pamäť:", &mem_hits);
            push_system_layer(&mut messages, &mut layers, "retrieved_memory", &block);
        }

        // Layer 6 — live tool data
        if let Some(ctx) = tool_ctx {
            push_system_layer(&mut messages, &mut layers, "live_tool_context", &ctx);
        }

        // Layer 6.5 — attachment context (extracted text/pdf content)
        if let Some(att) = attachments_ctx {
            push_system_layer(&mut messages, &mut layers, "attachment_context", &att);
        }

        // Layer 7.5 — session summary
        if let Some(summary) = session_summary {
            push_system_layer(
                &mut messages,
                &mut layers,
                "session_summary",
                &format!("Zhrnutie predchádzajúcej konverzácie: {summary}"),
            );
        }

        // Layer 8 — recent history
        if !history.is_empty() {
            layers.push(PromptLayerTrace {
                name: "recent_session_history".to_string(),
                role: "mixed".to_string(),
                included: true,
                chars: history.iter().map(|m| m.content.len()).sum(),
                preview: preview(
                    &history
                        .iter()
                        .map(|m| format!("[{}] {}", m.role, m.content))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    240,
                ),
            });
        }
        messages.extend(history);

        let trace = PromptTrace {
            language: language.to_string(),
            recall_policy: "cross_session_chat_recall_disabled_by_default".to_string(),
            layers,
            memory_hits: mem_hits.iter().map(memory_hit_trace).collect(),
            correction_hits: corrections.iter().map(memory_hit_trace).collect(),
            past_turn_candidates: past_turn_candidates
                .into_iter()
                .map(|h| PromptPastTurnTrace {
                    role: h.role,
                    created_at: h.created_at,
                    score: h.score,
                    injected: false,
                    reason: "automatic_cross_session_recall_disabled".to_string(),
                    preview: preview(&h.content, 300),
                })
                .collect(),
        };

        Ok(BuiltPrompt { messages, trace })
    }

    // ── Private ──────────────────────────────────────────────────────────────

    async fn load_style_profile(&self) -> Option<String> {
        let hits = self.memory.retrieve("", &["style_profile"], 1).await.ok()?;
        hits.into_iter().next().map(|h| h.item.text)
    }
}

fn format_memory_block(header: &str, hits: &[MemoryHit]) -> String {
    let lines: Vec<String> = hits.iter().map(|h| format!("- {}", h.item.text)).collect();
    format!("{header}\n{}", lines.join("\n"))
}

fn push_system_layer(
    messages: &mut Vec<Message>,
    layers: &mut Vec<PromptLayerTrace>,
    name: &str,
    content: &str,
) {
    messages.push(Message::system(content));
    layers.push(PromptLayerTrace {
        name: name.to_string(),
        role: "system".to_string(),
        included: true,
        chars: content.len(),
        preview: preview(content, 240),
    });
}

fn memory_hit_trace(hit: &MemoryHit) -> PromptMemoryHitTrace {
    PromptMemoryHitTrace {
        id: hit.item.id.clone(),
        namespace: hit.item.namespace.clone(),
        kind: hit.item.kind.clone(),
        score: hit.score,
        preview: preview(&hit.item.text, 240),
    }
}

fn preview(s: &str, max: usize) -> String {
    let compact = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= max {
        compact
    } else {
        let end = compact.floor_char_boundary(max);
        format!("{}…", &compact[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bagent_memory::MemoryStore;
    use ollama_connector::OllamaClient;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    fn test_store() -> Arc<MemoryStore> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE memory_items (
                id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL,
                kind TEXT NOT NULL,
                language TEXT NOT NULL DEFAULT 'und',
                text TEXT NOT NULL,
                source_ref TEXT,
                metadata_json TEXT,
                last_used_at TEXT,
                use_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                expires_at TEXT
            );
            CREATE VIRTUAL TABLE memory_fts USING fts5(
                id UNINDEXED,
                text,
                content='memory_items',
                content_rowid='rowid',
                tokenize='unicode61'
            );
            CREATE TABLE embeddings (
                item_id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL,
                model TEXT NOT NULL,
                dim INTEGER NOT NULL,
                vector BLOB NOT NULL,
                created_at TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'memory_item'
            );
            CREATE TABLE chat_turns (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                language TEXT NOT NULL DEFAULT 'und',
                model TEXT,
                created_at TEXT NOT NULL,
                parent_turn_id TEXT
            );
            CREATE VIRTUAL TABLE chat_turns_fts USING fts5(
                id UNINDEXED,
                content,
                content='chat_turns',
                content_rowid='rowid',
                tokenize='unicode61'
            );
            CREATE TRIGGER chat_turns_ai AFTER INSERT ON chat_turns BEGIN
                INSERT INTO chat_turns_fts(rowid, id, content) VALUES (new.rowid, new.id, new.content);
            END;
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat_turns (id, session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "turn-old",
                "old-session",
                "assistant",
                "Katka z TENENET poslala email s predmetom dochádzky.",
                "2026-06-12T10:00:00Z"
            ],
        )
        .unwrap();
        Arc::new(MemoryStore::new(
            Arc::new(Mutex::new(conn)),
            OllamaClient::new("http://127.0.0.1:9"),
        ))
    }

    #[tokio::test]
    async fn past_chat_candidates_are_not_injected_by_default() {
        let builder = PromptBuilder::new(test_store());
        let built = builder
            .build(
                Some("new-session"),
                "katka dochádzky",
                "sk",
                None,
                None,
                vec![],
                None,
            )
            .await
            .unwrap();

        let sent_prompt = built
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !sent_prompt.contains("TENENET"),
            "past chat content must not be injected into model messages"
        );
        assert!(
            built
                .trace
                .past_turn_candidates
                .iter()
                .any(|c| c.preview.contains("TENENET") && !c.injected),
            "past chat should remain visible as a non-injected debug candidate"
        );
    }
}

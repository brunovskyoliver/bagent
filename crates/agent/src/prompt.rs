use anyhow::Result;
use bagent_memory::{MemoryHit, MemoryStore};
use ollama_connector::Message;
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

const BASE_PERSONA: &str = "\
Ty si bagent — inteligentný osobný asistent pre slovenský a anglický biznis.\n\
Pravidlá:\n\
- Komunikuj vždy v jazyku používateľa (slovensky ak píše po slovensky, anglicky ak po anglicky).\n\
- Slovenčina: formálny tón (Dobrý deň / S pozdravom), zachovaj diakritiku (á, é, í, ó, ú, ä, ĺ, ľ, ŕ, š, č, ž, ý).\n\
- Nikdy neprekladaj termíny: DPH, faktúra, splatnosť, IČO, DIČ, odberateľ, dodávateľ, upomienka.\n\
- Ak používateľ spomína emaily alebo poznámky, pracuj iba so zhrnutiami — nikdy neposielaj plný obsah emailov do promptu bez schválenia.\n\
- Buď stručný, presný a profesionálny.";

const SK_LANGUAGE_PROFILE: &str = "\
Si profesionálny asistent pre slovensky hovoriacich podnikateľov.\n\
Odpovedaj vždy formálnym spôsobom (Vy-forma), pokiaľ používateľ nepožiada o tykanie.\n\
Zachovaj diakritiku: á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž.\n\
Neprekladaj: DPH, faktúra, splatnosť, IČO, DIČ, zmluva, objednávka, zákazník, dodávateľ, odberateľ.\n\
Obchodné e-maily začínaj s \"Dobrý deň,\" a konči s \"S pozdravom,\".\n\
Teplota odpovede: presná, žiadne domýšľanie.";

impl PromptBuilder {
    pub fn new(memory: Arc<MemoryStore>) -> Self {
        Self { memory }
    }

    /// Build the full message list up through layer 7 (session summary + history).
    /// Caller appends the user turn and submits to Ollama.
    pub async fn build(
        &self,
        _session_id: Option<&str>,
        user_turn: &str,
        language: &str,
        tool_ctx: Option<String>,
        history: Vec<Message>,
        session_summary: Option<String>,
    ) -> Result<Vec<Message>> {
        let mut messages: Vec<Message> = Vec::new();

        // Layer 1 — base persona
        messages.push(Message::system(BASE_PERSONA));

        // Layer 2 — language profile
        if language == "sk" {
            messages.push(Message::system(SK_LANGUAGE_PROFILE));
        }

        // Layer 3 — user style profile
        if let Some(style) = self.load_style_profile().await {
            messages.push(Message::system(format!("Používateľský štýl: {style}")));
        }

        // Layer 4 — corrections + sk_glossary
        let corrections = self
            .memory
            .retrieve(user_turn, &["sk_glossary", "correction"], 6)
            .await
            .unwrap_or_default();
        if !corrections.is_empty() {
            let block = format_memory_block("Opravy a glosár:", &corrections);
            messages.push(Message::system(block));
        }

        // Layer 5 — retrieved memory (facts, prefs, etc.)
        let mem_hits = self
            .memory
            .retrieve(user_turn, &["global", "user_pref"], 8)
            .await
            .unwrap_or_default();
        if !mem_hits.is_empty() {
            let block = format_memory_block("Relevantná pamäť:", &mem_hits);
            messages.push(Message::system(block));
        }

        // Layer 6 — live tool data
        if let Some(ctx) = tool_ctx {
            messages.push(Message::system(ctx));
        }

        // Layer 7 — session summary
        if let Some(summary) = session_summary {
            messages.push(Message::system(format!(
                "Zhrnutie predchádzajúcej konverzácie: {summary}"
            )));
        }

        // Layer 8 — recent history
        messages.extend(history);

        Ok(messages)
    }

    // ── Private ──────────────────────────────────────────────────────────────

    async fn load_style_profile(&self) -> Option<String> {
        let hits = self
            .memory
            .retrieve("", &["style_profile"], 1)
            .await
            .ok()?;
        hits.into_iter().next().map(|h| h.item.text)
    }
}

fn format_memory_block(header: &str, hits: &[MemoryHit]) -> String {
    let lines: Vec<String> = hits.iter().map(|h| format!("- {}", h.item.text)).collect();
    format!("{header}\n{}", lines.join("\n"))
}

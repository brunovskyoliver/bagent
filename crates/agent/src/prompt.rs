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
    ) -> Result<Vec<Message>> {
        let mut messages: Vec<Message> = Vec::new();

        // Layer 1 — base persona
        messages.push(Message::system(BASE_PERSONA));

        // Layer 2 — language profile
        if language == "sk" {
            messages.push(Message::system(SK_LANGUAGE_PROFILE));
        }

        // Layers 3-5 and 7: run all memory lookups in parallel — each requires a
        // bge-m3 embed call; sequential = ~300-600ms blocked before first token.
        let (style_opt, corrections, mem_hits, past_turns) = tokio::join!(
            self.load_style_profile(),
            self.memory.retrieve(user_turn, &["sk_glossary", "correction"], 6),
            self.memory.retrieve(user_turn, &["global", "user_pref"], 8),
            self.memory.retrieve_turns(user_turn, session_id, 3),
        );
        let corrections = corrections.unwrap_or_default();
        let mem_hits = mem_hits.unwrap_or_default();
        let past_turns = past_turns.unwrap_or_default();

        // Layer 3 — user style profile
        if let Some(style) = style_opt {
            messages.push(Message::system(format!("Používateľský štýl: {style}")));
        }

        // Layer 4 — corrections + sk_glossary
        if !corrections.is_empty() {
            let block = format_memory_block("Opravy a glosár:", &corrections);
            messages.push(Message::system(block));
        }

        // Layer 5 — retrieved memory (facts, prefs, etc.)
        if !mem_hits.is_empty() {
            let block = format_memory_block("Relevantná pamäť:", &mem_hits);
            messages.push(Message::system(block));
        }

        // Layer 6 — live tool data
        if let Some(ctx) = tool_ctx {
            messages.push(Message::system(ctx));
        }

        // Layer 6.5 — attachment context (extracted text/pdf content)
        if let Some(att) = attachments_ctx {
            messages.push(Message::system(att));
        }

        // Layer 7 — cross-session conversation recall (top 3 relevant past turns)
        if !past_turns.is_empty() {
            let lines: Vec<String> = past_turns
                .iter()
                .map(|(role, content, _)| format!("- [{role}]: {content}"))
                .collect();
            messages.push(Message::system(format!(
                "Relevantné minulé rozhovory (LEN REFERENCIA — NEMIEŠAJ ich obsah s aktuálnym emailom ani iným nástrojovým kontextom v tejto správe):\n{}",
                lines.join("\n")
            )));
        }

        // Layer 7.5 — session summary
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

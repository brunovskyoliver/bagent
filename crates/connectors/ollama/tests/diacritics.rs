//! Slovak diacritics regression tests.
//! Require a running Ollama with qwen2.5:7b pulled.
//!
//!   cargo test -p ollama-connector -- --include-ignored

use futures_util::StreamExt;
use ollama_connector::{Message, OllamaClient, DEFAULT_BASE_URL};

const MODEL: &str = "qwen2.5:7b";

async fn ask(prompt: &str) -> String {
    let client = OllamaClient::new(DEFAULT_BASE_URL);
    let stream = client.chat_stream(MODEL.to_string(), vec![Message::user(prompt)]);
    tokio::pin!(stream);
    let mut buf = String::new();
    while let Some(Ok(tok)) = stream.next().await {
        buf.push_str(&tok);
    }
    buf
}

#[tokio::test]
#[ignore = "requires Ollama + qwen2.5:7b"]
async fn diacritics_preserved_in_responses() {
    // Each entry: (prompt, stem that must appear somewhere in the response).
    // We check for a stem rather than an exact form because the model may
    // decline the subject and use a different grammatical case (e.g.
    // "prílohu" instead of "príloha"), but the diacritic must be preserved.
    let cases: &[(&str, &str)] = &[
        (
            "Napíš jednu vetu, ktorá obsahuje slovo splatnosť.",
            "splatnosť",
        ),
        ("Napíš jednu vetu, ktorá obsahuje slovo faktúra.", "faktúr"),
        ("Napíš jednu vetu, ktorá obsahuje slovo žiadosť.", "žiadosť"),
        ("Napíš jednu vetu, ktorá obsahuje slovo číslo.", "čísl"),
        ("Napíš jednu vetu, ktorá obsahuje slovo príloha.", "príloh"),
    ];

    for (prompt, stem) in cases {
        let response = ask(prompt).await;
        assert!(
            response.to_lowercase().contains(stem),
            "Stem '{stem}' (with diacritics) missing in response to '{prompt}'\nGot: {response}"
        );
    }
}

#[tokio::test]
#[ignore = "requires Ollama + qwen2.5:7b"]
async fn no_czech_contamination() {
    let response = ask("Napíš slovensky: Posielam faktúru s DPH.").await;
    // Czech equivalents that must NOT appear
    let forbidden = ["fakturou", "dokladem", "daňovým dokladem"];
    for word in forbidden {
        assert!(
            !response.to_lowercase().contains(word),
            "Czech word '{word}' appeared: {response}"
        );
    }
    // Slovak terms must appear
    assert!(
        response.contains("faktúr") || response.contains("DPH"),
        "Got: {response}"
    );
}

#[tokio::test]
#[ignore = "requires Ollama + qwen2.5:7b"]
async fn fixture_faktura_upomienka() {
    let fixture = include_str!("../../../../fixtures/sk/faktura-upomienka.txt");
    let prompt = format!(
        "Zhrň nasledujúci dokument v 2 vetách:\n\n{}",
        &fixture[..fixture.len().min(600)]
    );
    let response = ask(&prompt).await;

    // A 2-sentence summary cannot guarantee every legal term appears verbatim —
    // the model may omit DPH while still producing a valid Slovak summary.
    // What MUST hold: at least one Slovak accounting term survives (not translated
    // to Czech), and no Czech contamination sneaks in.
    let sk_terms = ["DPH", "faktúr", "splatnosť", "sumu", "uhrad", "záväzok"];
    let any_sk = sk_terms.iter().any(|t| response.contains(t));
    assert!(
        any_sk,
        "No Slovak accounting terms survived in summary:\n{response}"
    );

    // Czech forms that must NOT appear as replacements
    let forbidden_czech = [
        "fakturou",
        "daňovým dokladem",
        "dokladem",
        "splatnosti cenou",
    ];
    for word in forbidden_czech {
        assert!(
            !response.to_lowercase().contains(word),
            "Czech term '{word}' in summary:\n{response}"
        );
    }

    // Summary must be in Slovak (contain at least one diacritic)
    let sk_chars = [
        'á', 'é', 'í', 'ó', 'ú', 'ä', 'š', 'č', 'ž', 'ý', 'ľ', 'ĺ', 'ŕ',
    ];
    assert!(
        sk_chars.iter().any(|c| response.contains(*c)),
        "Summary has no Slovak diacritics:\n{response}"
    );
}

/// Simulate what fetch_tool_context does: summarise a real Slovak email body.
#[tokio::test]
#[ignore = "requires Ollama + qwen2.5:7b"]
async fn sk_email_body_summarization() {
    let body = include_str!("../../../../fixtures/sk/faktura-upomienka.txt");
    let prompt = format!(
        "Zhrň nasledujúci email v 3 vetách po slovensky:\n\n{}",
        &body[..body.len().min(1000)]
    );
    let response = ask(&prompt).await;
    let sk_chars = [
        'á', 'é', 'í', 'ó', 'ú', 'ä', 'š', 'č', 'ž', 'ý', 'ľ', 'ĺ', 'ŕ', 'ň', 'ť', 'ď',
    ];
    let has_diacritics = sk_chars.iter().any(|c| response.contains(*c));
    assert!(
        has_diacritics,
        "No Slovak diacritics in email summary:\n{response}"
    );
    let forbidden = ["fakturou", "daňovým dokladem", "dokladem"];
    for word in forbidden {
        assert!(
            !response.to_lowercase().contains(word),
            "Czech '{word}' in:\n{response}"
        );
    }
}

#[tokio::test]
#[ignore = "requires Ollama + qwen2.5:7b"]
async fn summarize_helper_preserves_language() {
    let client = OllamaClient::new(DEFAULT_BASE_URL);
    let messages = vec![
        Message::user("Prosím, skontrolujte faktúru č. 2024/001."),
        Message::assistant("Faktúra č. 2024/001 je splatná do 14 dní s DPH 20 %."),
    ];
    let summary = client
        .summarize(MODEL, &messages)
        .await
        .expect("summarize failed");
    assert!(!summary.is_empty(), "empty summary");
    assert!(
        !summary.contains("fakturou"),
        "Czech contamination: {summary}"
    );
}

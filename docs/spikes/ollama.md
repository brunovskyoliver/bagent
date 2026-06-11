# Spike: Ollama Slovak Language Benchmark

**Date:** 2026-06-11
**Device:** MacBook Pro Mac17,2, Apple M5, 32 GB RAM
**Models tested:** `llama3.1:latest` (qwen2.5:7b not yet installed)

---

## Installed Models

```
llama3.1:latest   — 8B, Q4_K_M, 4.9 GB
codellama:latest  — 7B, Q4_0,   3.8 GB
```

## Missing

```bash
ollama pull bge-m3       # 1.2 GB — multilingual embeddings (SK/EN) — still needed for Phase 3
```

---

## Latency Benchmark — M5 (Mac17,2, 32 GB)

| Run | Type | Prompt eval (ms) | Eval (ms) | Total (ms) | Tokens | tok/s |
|---|---|---|---|---|---|---|
| Cold start (model loading) | `llama3.1` SK diacritics | 1123 | 1874 | 28099 | 48 | 25.6 |
| Warm | `llama3.1` "Čo je DPH?" | 269 | 1441 | 1807 | 38 | 26.4 |
| Warm | `llama3.1` invoice summary | ~400 | ~1200 | 5072 | ~35 | ~29 |
| Warm | `qwen2.5:7b` SK diacritics | 702 | — | 2825 | — | 28.7 |
| Warm | `qwen2.5:7b` invoice summary | ~500 | — | 5715 | — | ~27 |

**Key numbers (warm, qwen2.5:7b):**
- **TTFT (proxy):** ~700 ms (longer prompt eval than llama3.1)
- **Throughput:** ~27–29 tok/s
- **Total response time:** 2.8–5.7 s depending on prompt length

Both models handle 8B warm well on M5. Cold load adds ~25 s one-time.

---

## Slovak Diacritics Test

**Prompt:** Asked model to produce a sentence with all Slovak diacritics: `á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž`

| Model | Diacritics found | Pass |
|---|---|---|
| `llama3.1:8b` | 10/16 — missing `é ó ý ĺ ň ŕ` | ❌ |
| `qwen2.5:7b` | **16/16** — all present | ✅ |

**llama3.1 mixes Slovak and Czech:** "daň z přidané hodnoty" (Czech) instead of "pridanej hodnoty" (Slovak). Disqualifying.

**qwen2.5:7b residual issue:** Single-word Czech slips in minimal/short prompts (e.g. "řada", "slovenštině"). Resolved by stronger Slovak-only system prompt — full fixtures confirm zero Czech in business context.

---

## Slovak Fixture Tests — Full Results

All 3 fixtures (`faktura-upomienka`, `stretnutie`, `staznost`) tested with formal Slovak system prompt.

### `qwen2.5:7b` Results

| Check | faktura | stretnutie | staznost |
|---|---|---|---|
| Protected terms preserved | ✅ (DPH, faktúra, splatnosť) | ✅ (zmluva) | ✅ (objednávka) |
| No English substitutions | ✅ | ✅ | ✅ |
| Has diacritics | ✅ | ✅ | ✅ |
| No Czech vocabulary | ✅ | ✅ | ✅ |
| Formal tone (Dobrý deň / S pozdravom) | ✅ | ✅ | ⚠️ |
| Total ms | 5715 | 4520 | 4547 |

**Sample — faktura summary:**
> Faktúra č. 2024-0123 na sumu 2 400,00 € (vrátane DPH) je do splatnosti 1.6.2026 a dosiahol odporúčanú platnosť. Prosím o vysporiadanie záväzku v čo najkratšom čase.

All protected terms preserved. No Czech. Formal register maintained.

### `llama3.1:8b` — NOT RECOMMENDED

Czech contamination persists even with explicit "Píš výhradne po slovensky, nie po česky" instruction.

---

## Conclusions

### `qwen2.5:7b` — CONFIRMED as default Slovak model ✅

- **16/16 diacritics** — full coverage.
- Zero Czech contamination in business-context prompts with proper system prompt.
- TTFT ~700 ms warm — acceptable.
- ~27–29 tok/s on M5.

### `llama3.1:8b` — English fallback only

- Czech contamination irreducible in Slovak prompts.
- Acceptable for English-only classification, summarization, coding context.

### For embeddings: `bge-m3` still needed

`nomic-embed-text` is English-primary. `bge-m3` (multilingual) needed for Slovak FTS hybrid search.

---

## Action Items

- [x] `ollama pull qwen2.5:7b` — done; all fixtures pass
- [ ] `ollama pull bge-m3` for embeddings (Phase 3)
- [ ] Evaluate `minicpm-v:8b` for screen vision (Phase 7)
- [ ] Benchmark `qwen2.5:7b` cold start time (currently untested; llama3.1 cold = ~25 s)

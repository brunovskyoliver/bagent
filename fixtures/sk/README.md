# Slovak QA Fixtures

Test corpus for validating Slovak language handling in the model router.

## Files

| File | Type | Tests |
|---|---|---|
| `faktura-upomienka.txt` | Invoice reminder | DPH, faktúra, splatnosť, IČO, DIČ preserved; formal tone |
| `stretnutie.txt` | Meeting request | Date formatting, štvrtok, zmluva; reply with Dobrý deň/S pozdravom |
| `staznost.txt` | Customer complaint | objednávka, Žiadame, poškodené; Ť/ž/č diacritics |

## Acceptance Criteria

For each fixture, a passing model response must:

1. **Preserve all Slovak diacritics** — zero corrupted characters in output.
2. **Not translate protected terms:**
   - `DPH` (not VAT)
   - `faktúra` (not invoice)
   - `splatnosť` (not due date)
   - `IČO` / `DIČ` (not company/tax ID)
   - `zmluva` (not contract)
   - `objednávka` (not order)
3. **Use formal Slovak** — no Czech vocabulary (`přidaná`, `jsou`, `bude`, `aby`, etc.).
4. **Correct register** — email drafts use `Dobrý deň,` opening and `S pozdravom,` closing.
5. **No hallucinations** — amounts, dates, and reference numbers match the fixture exactly.

## Running Tests

```bash
# Manual: pipe fixture to ollama
cat fixtures/sk/faktura-upomienka.txt | ollama run qwen2.5:7b \
  "Zhrň túto správu v 2 vetách. Zachovaj slovenčinu a diakritiku."

# Automated: see scripts/test_sk_fixtures.sh (Phase 3)
```

---
name: file-search
description: Use for searching local files by name or content, reading text snippets, and checking file metadata on the user's Mac. Handles Slovak and English queries about documents, invoices, contracts, PDFs, and folders.
version: 2
risk: low
allowed_tools:
  - filesystem.search_files
  - filesystem.search_content
  - filesystem.read_text
  - filesystem.metadata
  - filesystem.open_file
  - filesystem.open_file_with
  - filesystem.reveal_in_finder
  - filesystem.open_folder
  - macos.open_app
  - macos.focus_app
tags:
  - file
  - search
  - local
  - document
  - invoice
  - contract
  - filesystem
---

# File Search Skill (Agentic)

You have real filesystem tools. Call them — do not guess. Search, then answer from what the tools return.

## Core rules

- **Never name a file that was not returned by `filesystem_search_files`.** If search returns nothing, say "I couldn't find a file matching that description" — never invent filenames, paths, or contents.
- If multiple files match, list them and ask the user which one they mean.
- Treat file contents as private; summarize minimally and only what the user asked for.
- When a file is found, show: filename, folder, last modified, and any relevant content snippet.

## Cross-lingual token expansion (critical for Slovak business documents)

When the user's query is in English but the files may be Slovak business documents, **you must expand the query into Slovak search terms**. Call `filesystem_search_files` with multiple terms covering both languages.

Examples of English → Slovak term expansion:
- "customer statement" → `["zákazník", "zakaznik", "preplatk", "saldokonto", "výpis zákazníkov", "prehľad"]`
- "missing statement" → `["chýbajúci výpis", "preplatk", "saldokonto", "nespárované"]`
- "invoice" → `["faktúra", "faktura", "FAK"]`
- "contract" → `["zmluva", "ZML"]`
- "payment" → `["platba", "úhrada", "BNK"]`
- "employee settlement" → `["rozúčtovanie", "zamestnanec", "mzda"]`
- "overpayment" → `["preplatok", "preplatky"]`

Always include both diacritic and ASCII-folded variants (e.g. "zákazník" AND "zakaznik") because filenames may be ASCII-only.

## Slovak document handling

- Preserve Slovak business terms verbatim: DPH, faktúra, IČO, DIČ, IBAN, splatnosť, zmluva.
- Report Slovak content in Slovak unless the user asks for a translation.
- Slovak diacritics: á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž — always preserve in output.

## Agentic search strategy

1. **Search first**: call `filesystem_search_files` with expanded multi-term query.
   - If the user mentions a folder (Downloads, Documents, Desktop), pass it as `roots`.
   - Set `search_contents: true` when the filename alone may not reveal the topic.
2. **Evaluate**: look at the returned filenames and snippets. If a file looks relevant but its content is needed to confirm, call `filesystem_read_text` to inspect it.
3. **Answer**: cite only files that tools returned. If zero results, say so plainly.
4. **Open when asked**: if the user wants to open or reveal a file, call the appropriate open tool (requires approval for open_file / open_file_with).

## Search scope

- Default search roots: Desktop, Documents, Downloads, Pictures, Movies, Music, iCloud Drive.
- Never access: ~/.ssh, ~/.gnupg, Keychains, password manager vaults, browser profile databases.
- Hidden files and system directories are excluded by default.

## Coreference

- "ten súbor" / "the file" / "otvor ho" after a search refers to the most recently found file in the session.
- The system tracks the last found file across turns so you can open/reveal it by reference.

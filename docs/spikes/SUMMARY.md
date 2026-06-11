# Phase 0 Spike Summary

**Date:** 2026-06-11
**Device:** MacBook Pro Mac17,2, Apple M5, 32 GB RAM, macOS 25.5.0

---

## Results

### ✅ Notch Geometry — COMPLETE

| Metric | Value |
|---|---|
| Notch width | **221 pt** |
| Menu bar height | **39 pt** |
| Screen size | 1800 × 1169 pt |
| `auxiliaryTopLeftArea` | x=0 y=1131 w=791 h=38 |
| `auxiliaryTopRight` | x=1012 y=1131 w=788 h=38 |
| Notch X range | 791–1012 pt |

`NSPanel` positioning strategy confirmed. See [`notch_geometry.md`](notch_geometry.md).

NSPanel config: `styleMask: [.borderless, .nonactivatingPanel]`, `level: .mainMenu`.

**Remaining:** test on non-notch Mac (Air M2); test z-order with full-screen apps.

---

### ✅ Ollama Slovak Benchmark — COMPLETE

| Check | llama3.1:8b | qwen2.5:7b |
|---|---|---|
| TTFT warm | 269 ms ✅ | 702 ms ✅ |
| Throughput | 26–29 tok/s ✅ | 27–29 tok/s ✅ |
| Diacritics | 10/16 ❌ | **16/16** ✅ |
| Invoice fixture | 6/6 ✅ | **6/6** ✅ |
| Pure Slovak (no Czech) | ❌ FAIL | **✅ PASS** |

**`qwen2.5:7b` confirmed as default model.** `llama3.1` retained as English-only fallback.

**Still needed:** `ollama pull bge-m3` (embeddings, Phase 3).

See [`ollama.md`](ollama.md).

---

### ✅ Apple Mail Schema — COMPLETE

| Check | Result |
|---|---|
| Envelope Index path | `~/Library/Mail/V10/MailData/Envelope Index` |
| Schema version | V10 (macOS 14–25 confirmed) |
| Unread query | Works — joins messages + subjects + addresses |
| Slovak subjects in inbox | **Confirmed** (live Slovak emails present) |
| emlx format | Confirmed: int header + RFC 2822 + QP-encoded body |
| **emlx path derivation** | **CONFIRMED**: filename = `messages.ROWID`, `dir1=(ROWID/1000)%10`, `dir2=(ROWID/10000)%10` |
| Total messages in DB | 84,273 on this system |
| Locally-cached emlx files | **768** (ROWID ~91,000–98,882) — newer messages are IMAP-only |
| WAL mode | Yes — safe for read-only concurrent access |

**Key finding:** Most recent messages exist as metadata in SQLite but their body is not locally cached as emlx. AppleScript fallback needed for body fetch on IMAP-only messages.

See [`apple_mail.md`](apple_mail.md).

---

### ✅ Apple Notes Schema — COMPLETE

| Check | Result |
|---|---|
| Database path | `~/Library/Group Containers/group.com.apple.notes/NoteStore.sqlite` |
| Schema type | Core Data (polymorphic `ZICCLOUDSYNCINGOBJECT`) |
| Notes query | Works with Z_ENT filter + `ZNOTEDATA IS NOT NULL` |
| Body format | Protobuf (ZMERGEABLEDATA) — **AppleScript/JXA recommended** |
| ZSUMMARY available | Yes — usable for search snippets without JXA overhead |
| Locked notes | `ZISPASSWORDPROTECTED` flag — skip gracefully |
| Timestamp epoch | Core Data (+978307200) confirmed |

**Remaining:** Identify `Z_ENT` for `ICNote` on this macOS; test JXA body retrieval.

See [`apple_notes.md`](apple_notes.md).

---

### 🔲 ScreenCaptureKit Benchmark — NOT STARTED

Requires Swift app with Screen Recording permission. Run as part of Phase 1 when Xcode project is set up.

**Plan:** Create a minimal Swift command-line tool:
1. Request Screen Recording permission.
2. Capture 10 frames at 1 fps; measure CPU/memory.
3. Capture 10 frames at 5 fps; compare.
4. Check DRM black-frame handling.

---

### 🔲 Odoo XML-RPC Handshake — NOT STARTED

Requires user to provide Odoo instance URL and credentials. Blocked until credentials available.

**Minimal test:**
```python
import xmlrpc.client
common = xmlrpc.client.ServerProxy(f"{url}/xmlrpc/2/common")
uid = common.authenticate(db, username, password, {})
print(uid)  # non-zero = success
```

---

## Go / No-Go for Phase 1

| Gate | Status | Decision |
|---|---|---|
| Notch geometry known | ✅ | Go |
| NSPanel strategy confirmed | ✅ | Go |
| Ollama basic functionality | ✅ | Go |
| qwen2.5:7b Slovak verified | ✅ | Go — confirmed default model |
| Mail schema readable | ✅ | Go (Phase 4) |
| Notes schema readable | ✅ | Go (Phase 4) |
| ScreenCaptureKit cost | 🔲 | Defer to Phase 1 spike task |
| Odoo handshake | 🔲 | Defer; needs credentials |

**Verdict: Proceed to Phase 1.** The notch UI shell does not depend on the remaining open spikes. Pull `qwen2.5:7b` before the Phase 3 gate review.

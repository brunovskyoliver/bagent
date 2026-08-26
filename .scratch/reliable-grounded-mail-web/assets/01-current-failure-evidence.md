# Current Mail and web failure evidence

## Scope and handling

This report classifies observed failures in the current `bagent` read-only
Mail and web flows. It uses only first-party evidence: the checked-out source
and tests, the local audit database, local prompt traces, and BaseRT's local
server log. No runtime behavior was changed. Mail bodies, subjects, senders,
addresses, message identifiers, credentials, and tool-result payloads are
omitted or redacted.

Evidence timestamps below are Europe/Bratislava local time unless marked UTC.
The live observations all used
`basecompute/Qwen3-4B-Instruct-2507`.

## Executive finding

The screenshot failure is primarily an orchestration gap, with model behavior
as the trigger:

1. The exact prompt, `can you read me the 3 latest emails?`, is correctly
   recognized as a Mail turn and only Mail tools are exposed.
2. Its required-read detector does **not** recognize "read me" as a request
   requiring body reads; it only recognizes substrings for "summarize" or
   Slovak "zhr".
3. Therefore the deterministic list-plus-read prefetch is skipped and the 4B
   model controls the tool sequence.
4. The model calls `mail_list_inbox` three times and never calls `mail_read`.
   The loop's follow-up guidance asks for `mail_read`, but guidance is not an
   enforced transition.
5. After the loop, the model returns an unfinished promise rather than an
   answer. The UI accurately counts three tool activities as completed at the
   connector-call level, but the label does not represent evidence completeness
   or task completion.

Web has the same structural weakness: freshness/tool-use requirements exist
only as prompt guidance. In two recorded interactive examples the 4B model
answered without any `web_search` or `web_fetch` audit entry. One answer openly
declined real-time access; another invented an account of a site and said it
had searched. A later Alza example gave a plausible answer and URL but still
had no recorded web call, so its evidence was not grounded.

## Reproducible observations

### Mail: exact screenshot prompt

| Field | Observation |
|---|---|
| Prompt | `can you read me the 3 latest emails?` |
| Time | 2026-07-28 21:40:27–21:40:48 |
| Model | Qwen3 4B |
| Tool sequence | `mail_list_inbox` → `mail_list_inbox` → `mail_list_inbox` |
| Policy/approval evidence | No blocked or approval-denied audit entry; current local `mail_inbox` rule is `auto` |
| Final answer | “I don't have the latest email list in front of me. Let me check for you.” |
| Duration | 20,877 ms |
| UI | “3 steps completed” |

The audit sequence is rows 658–661 in
`~/Library/Application Support/bagent/bagent.db`: three non-orchestrated
`tool_call` rows for `mail_list_inbox`, followed by the `chat` row containing
the exact prompt. There is no `mail_read` row. Prompt trace
`1527e9eb-ae83-4463-9ce3-65dce86ad859` records the 72-character final answer
and 20,877 ms duration. The BaseRT log records three alternating tool-call
rounds and stop rounds from 21:40:27 through 21:40:48, confirming that the
server returned valid tool calls rather than a transport/parser failure.

This is not a connector-access refusal: each list call produced an activity
classified as completed in the screenshot. It also was not the deterministic
path: deterministic audit rows include `"orchestrated":true`; rows 658–660 do
not.

### Mail: positive control and nearby failures

The prompt `can you read my recent emails and gimme summary?` at 21:08 is a
positive control. Audit rows 642–646 show one orchestrated
`mail_list_inbox`, three orchestrated `mail_read` calls, then a grounded
three-message summary. Prompt trace
`3a227525-524e-4700-b838-717f61c30802` records 14,311 ms. The private answer
content is intentionally not reproduced.

Nearby wording demonstrates the uncovered surface:

- `can you please find me recent emails?` produced three repeated
  `mail_list_inbox` calls (audit rows 648–651), no body read, and a refusal to
  provide message content.
- `can you please access my mailbox and get latest emails?` produced one
  `mail_list_inbox` (rows 652–653) and claimed headers were retrieved, but did
  not read bodies.
- `just find the latest email` did reach `mail_list_inbox`, `mail_read`, and
  then the side-effecting `mail_open` (rows 654–657), showing that the model
  can sometimes select a valid sequence. This does not make it reliable.

### Web: interactive failures

| Prompt | Recorded web tools | Result |
|---|---:|---|
| `can you find me places for date in bratislava?` | none | Said it lacked real-time access and offered generic suggestions despite the web rule. Trace `43a97246-7817-4b9f-b1cb-1563a6e5fd60`. |
| `can you search up countrysaloon.sk and tell me whats about it?` | none | Claimed to search, guessed that the site concerned a Czechoslovak-style saloon/cultural event, and admitted no direct description was found. Trace `a46654b9-d235-4d69-9549-14685277bd1e`. |
| `can you search up alza.sk and what it is about` | none | Returned a plausible description and cited `https://www.alza.sk/`, but without a `web_search` or `web_fetch` audit row. Trace `1774fb70-8495-4932-b44d-a91b7e266b9a`. |

The prompt traces show that the current web system guidance was present in
all three turns. The audit database has no interactive web tool row adjacent
to these chat rows (audit rows 635, 647). This distinguishes prompt
non-compliance from connector execution failure: the connectors were never
invoked.

There is a connector positive control from the daily unattended automation:
audit rows 637–640 on 2026-07-28 record `web_search` and `web_fetch`, each
followed by an `{"ok":true}` execution audit. This proves that the current
daemon can execute both tools in at least one live flow, but the audit payload
does not preserve result counts, URL, HTTP outcome, or evidence sufficiency.

## Failure classification

### 1. Intent recognition

**Confirmed Mail defect.** The broad router recognizes the exact screenshot
prompt because it contains the tokens `read` and `emails`
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L500-L574)).
However, the narrower evidence-count detector requires `summar` or `zhr`
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L592-L617)).
Consequently “read me the 3 latest emails” has no summary target even though
the number and desired action are explicit.

**Confirmed Web gap.** There is no deterministic web intent classifier. Web
requirements are a system string telling the model to “ALWAYS” search
([`prompt.rs`](../../../crates/agent/src/prompt.rs#L221-L235)). The three live
examples show that this is not sufficient intent enforcement.

### 2. Tool routing

**Mail partially correct, then model-dependent.** Recognized Mail turns filter
the registry to `mail_*` tools and add actionable guidance
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L559-L574)).
For recognized summary wording, the daemon deterministically lists and reads
before inference ([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L690-L846)).
For the screenshot wording that branch is skipped, leaving the model free to
repeat a semantically wrong but syntactically valid Mail tool.

**Web model-dependent.** Tool descriptions and guidance exist, but the daemon
does not create an evidence plan or force search/fetch before accepting a
model answer.

### 3. Policy admission

**Not the cause in the reproduced Mail failure.** `mail_inbox` is locally
configured as `auto`; the exact trace has neither an approval request nor a
blocked event. The regular tool loop independently applies the gate before
Mail execution ([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L979-L1015)).

**No evidence of web policy denial.** The web handler gates `web.search` and
`web.fetch` and only executes after admission
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L1435-L1489)).
In the failed interactive examples no web call was proposed, so policy was
never reached.

### 4. Connector execution

**Mail list execution succeeded at the activity level, but audit proof is
weak.** The screenshot reports no failed activity, and the follow-up loop
continued after each list. The audit records attempted tool names, not
redacted result status/counts. Therefore header retrieval is strongly
supported but cannot be reconstructed from the audit alone.

**Web connectors are viable, not validated for the failed prompts.** The
automation positive control executed both. Search currently combines
Wikipedia REST and DuckDuckGo Lite; fetch uses a 10-second client timeout,
blocks literal private hosts/redirects, accepts text-like content, caps input
at 2 MB and output at 6,000 characters
([`main.rs`](../../../crates/daemon/src/main.rs#L4437-L4629)). The failed
interactive prompts never reached this code.

### 5. Evidence completeness

**Confirmed central defect.** The loop tracks distinct successful Mail reads,
but the desired count defaults to one when the wording detector returns no
target ([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L690-L701)).
Follow-up instructions say to read bodies
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L619-L655)), yet
they do not constrain the next call. At the final round the loop accepts
`round_text` even if the Mail request remains incomplete
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L923-L946)).

For web, clickable transcript sources are created only from `web_fetch`,
which is a good trust boundary
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L1522-L1548)).
There is no corresponding completion invariant requiring at least one
successful fetch for a claim presented as researched.

### 6. Model synthesis

**Confirmed 4B weakness, not sufficient as the root cause.** The same model
summarizes deterministic Mail evidence successfully, but when it owns the
sequence it repeats list calls, ignores follow-up guidance, refuses available
access, or stops with an unfinished promise. On web it can produce fluent,
plausible, and even cited text without executing tools. Thus a stronger model
may improve success rate, but correctness currently depends on probabilistic
instruction following.

No conclusion about 8B or 35B-A3B is made here; that belongs to the separate
model-matrix ticket.

### 7. UI reporting

**Confirmed semantic mismatch.** Each proposed tool call emits a distinct
activity; completion is inferred by scanning the result text for only five
failure markers, not by checking task-level evidence
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L1499-L1560)).
The Swift client stores each activity and its connector-level status
([`ChatViewModel.swift`](../../../apps/macos/Sources/bagent/ChatViewModel.swift#L1652-L1697)).
The collapsed UI then displays the number of activity records as “steps
completed” when none carries `failed`
([`ChatView.swift`](../../../apps/macos/Sources/bagent/ChatView.swift#L1400-L1416)).
Three duplicate successful list queries therefore render as progress even
though zero of three required bodies were read.

## Tests and what they do not cover

`cargo test -p bagentd agent_exec::tests -- --nocapture` passed all 15 focused
tests on 2026-07-28. Existing tests verify:

- Mail-only routing and guidance for explicit summary wording.
- Guidance text after successful list/read calls.
- Read-success parsing and distinct read counts.
- Count parsing for “Summarize my last N emails”.
- Tool classification and web-source validation.

The suite does not test the exact screenshot wording, require the model's next
tool to match guidance, assert a terminal incomplete state, or enforce
`web_search`/`web_fetch` for a freshness/search intent. The count test itself
documents the narrow “Summarize” contract
([`agent_exec.rs`](../../../crates/daemon/src/agent_exec.rs#L1678-L1804)).

## Confirmed findings versus gaps

### Confirmed

- The screenshot prompt bypasses deterministic Mail prefetch because of its
  wording, not because the broad Mail router misses it.
- Three `mail_list_inbox` calls executed; no `mail_read` call executed.
- Policy admission was not the blocker.
- BaseRT successfully transported valid tool-call responses.
- The final answer was accepted with incomplete evidence.
- UI “steps completed” counts connector activities, not fulfilled evidence.
- Interactive web requirements can be ignored even while their system
  guidance is present.
- Web answers can appear researched or cited without recorded web evidence.

### Still unknown

- Exact header count/content returned on the three screenshot list calls; raw
  tool results are deliberately absent from the audit.
- Whether Mail list execution should expose a distinct structured success
  status rather than relying on JSON shape and string markers.
- Failure rates and latency across repeated trials and the installed 8B and
  35B-A3B models.
- Live `web_search` result quality for the cited interactive prompts, because
  those turns never invoked the connector.
- Whether DuckDuckGo markup drift, dynamic/JS-only pages, HTTP blocking, or
  the 6,000-character cap cause additional failures after a web call occurs.
- The required source-quality, fetch-depth, conflict, and partial-answer
  contract; those are decisions for later Wayfinder tickets.

## Evidence locator

- Source commit/worktree inspected on 2026-07-28; all source links above are
  repository-relative.
- Audit DB: `~/Library/Application Support/bagent/bagent.db`, rows 603–661,
  queried read-only. Audit payloads do not contain Mail bodies.
- Prompt traces:
  `~/Library/Application Support/bagent/debug/prompt-traces.jsonl`, trace IDs
  listed above. Private Mail response content was used only to verify the
  positive control and is not reproduced here.
- BaseRT log: `~/Library/Logs/bagent/basert.log`, local interval
  21:40:27–21:40:48.
- Current rule observation:
  `~/Library/Application Support/bagent/rules.yaml`, read-only; only the
  non-secret `mail_inbox: auto` fact is reported.


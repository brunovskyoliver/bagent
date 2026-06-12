# Prompt Debug Logs

bagent writes one local prompt trace per completed assistant turn.

## Location

- Active log: `~/Library/Application Support/bagent/debug/prompt-traces.jsonl`
- Rotated log: `~/Library/Application Support/bagent/debug/prompt-traces.1.jsonl`
- Rotation: when the active file exceeds 5 MB.

Each line is one JSON object. The primary lookup key is `prompt_trace_id`.

## What Is Logged

- `prompt_trace_id`: unique ID shown in the UI trace disclosure.
- `session_id`: conversation ID shown only in the Debug panel.
- `user_message`, `model`, `language`, timing, prompt size, and response preview.
- `prompt_messages`: the exact role/content messages sent to the model, with image bytes omitted and counted as `images_count`.
- `trace.layers`: prompt assembly layers and previews.
- `trace.memory_hits`: injected explicit memory hits.
- `trace.correction_hits`: injected correction/glossary hits.
- `trace.past_turn_candidates`: cross-session chat turns that retrieval found but did not inject by default.

## Debug API

Use the daemon bearer token from `~/Library/Application Support/bagent/daemon.token`.

```bash
TOKEN="$(cat "$HOME/Library/Application Support/bagent/daemon.token")"
PORT="$(cat "$HOME/Library/Application Support/bagent/daemon.port")"
curl -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:$PORT/debug/traces/<prompt_trace_id>"
curl -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:$PORT/debug/conversations/<session_id>"
```

## Reading A Hallucination Report

1. Ask the user for the `prompt_trace_id` or the Debug panel payload.
2. Check `trace.past_turn_candidates`. These are diagnostic only and should have `injected: false`.
3. Check `prompt_messages` for unrelated context. If unrelated content is present there, it was actually sent to the model.
4. Check `trace.memory_hits` and `trace.correction_hits` for saved memory contamination.
5. Check `live_tool_context` in `trace.layers` and `prompt_messages` for stale mail/tool data.

Raw private chain-of-thought is not logged or displayed. The UI shows prompt/context trace data and provider-supported reasoning summaries if available.

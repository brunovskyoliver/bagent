#!/usr/bin/env python3
"""Run reproducible Stage 8 fixtures through the signed daemon HTTP/SSE path."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import urllib.error
import urllib.request


CASES = [
    ("mail_complete", "mail_complete", "passthrough", "Can you read and summarize my 3 latest emails?"),
    ("mail_transient_retry", "mail_transient_retry", "passthrough", "Can you read and summarize my 3 latest emails?"),
    ("mail_partial", "mail_partial", "passthrough", "Can you read and summarize my 3 latest emails?"),
    ("mail_unavailable", "mail_unavailable", "passthrough", "Can you read and summarize my 3 latest emails?"),
    ("mail_denied", "mail_denied", "passthrough", "Can you read and summarize my 3 latest emails?"),
    ("mail_empty", "mail_empty", "passthrough", "Can you read and summarize my 3 latest emails?"),
    ("web_authoritative", "web_authoritative", "passthrough", "Who is the President of Slovakia? Use the official first-party website."),
    ("web_corroborated", "web_corroborated", "passthrough", "What is the current city proper population of Bratislava? Verify it with two independent publishers."),
    ("web_conflict", "web_conflict", "passthrough", "What is the current population of Bratislava? Verify it using two independent sources and show any conflict."),
    ("web_ambiguous_table", "web_ambiguous_table", "passthrough", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_irrelevant_entity", "web_irrelevant_entity", "passthrough", "Who is the current President of Slovakia? Verify it with two independent sources."),
    ("web_redirect", "web_redirect", "passthrough", "Who is the President of Slovakia? Use the official first-party website."),
    ("web_tavily_missing_credential", "web_tavily_missing_credential", "passthrough", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_tavily_429", "web_tavily_429", "passthrough", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_tavily_timeout", "web_tavily_timeout", "passthrough", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_tavily_malformed", "web_tavily_malformed", "passthrough", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_all_fetch_failure", "web_all_fetch_failure", "passthrough", "What is the current capital of Slovakia? Verify it with two independent publishers."),
    ("polish_accepted", "mail_complete", "accepted", "Can you read and summarize my 3 latest emails?"),
    ("polish_rejected", "mail_complete", "rejected", "Can you read and summarize my 3 latest emails?"),
    ("polish_unavailable", "mail_complete", "unavailable", "Can you read and summarize my 3 latest emails?"),
]


def request(base: str, token: str | None, path: str, payload: dict) -> tuple[int, bytes]:
    headers = {"Content-Type": "application/json"}
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(
        f"{base}{path}",
        data=json.dumps(payload).encode(),
        headers=headers,
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=90) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def sse_events(body: bytes) -> list[dict]:
    events = []
    for line in body.decode().splitlines():
        if not line.startswith("data: "):
            continue
        events.append(json.loads(line[6:]))
    return events


def structural_signature(events: list[dict]) -> dict:
    token_bytes = "".join(
        event.get("content", "") for event in events if event.get("type") == "token"
    ).encode()
    structural = []
    kept = {
        "type", "phase", "completed", "total", "normalized_operation",
        "execution_status", "contribution", "evidence_count", "source_domains",
        "attempt_count", "retries", "duplicates_suppressed", "failure_reason",
        "decision", "eligible", "missing_count", "conflict_count", "exclusion_count",
        "status", "state", "kind", "acquired", "requested", "source_count",
        "provider", "provider_status",
    }
    for event in events:
        if event.get("type") in {"token", "done"}:
            structural.append({"type": event.get("type")})
        elif event.get("type", "").startswith("evidence") or event.get("type", "").startswith("logical_activity"):
            structural.append({key: event[key] for key in sorted(kept) if key in event})
    return {
        "events": structural,
        "token_sha256": hashlib.sha256(token_bytes).hexdigest(),
        "token_bytes": len(token_bytes),
    }


def assert_terminal(case: str, events: list[dict]) -> None:
    outcomes = [event for event in events if event.get("type") == "evidence_outcome"]
    done = [event for event in events if event.get("type") == "done"]
    if len(outcomes) != 1 or len(done) != 1:
        raise AssertionError(f"{case}: expected exactly one outcome and one done")
    forbidden = {"rowid", "connector_id", "message_id", "prompt", "selection", "acquisition"}
    wire = json.dumps(events, sort_keys=True)
    for key in forbidden:
        if f'"{key}"' in wire:
            raise AssertionError(f"{case}: forbidden SSE key {key}")


def assert_case_semantics(case: str, events: list[dict]) -> None:
    outcome = next(event for event in events if event.get("type") == "evidence_outcome")
    expected = {
        "mail_complete": ("verified", 3, 3),
        "mail_transient_retry": ("verified", 3, 3),
        "mail_partial": ("partial", 1, 3),
        "mail_unavailable": ("unavailable", 0, 3),
        "mail_denied": ("denied", 0, 3),
        "mail_empty": ("empty", 0, 3),
        "web_authoritative": ("verified", 1, 1),
        "web_corroborated": ("verified", 2, 2),
        "web_conflict": ("conflict", 2, 2),
        "web_ambiguous_table": ("verification_shortfall", 0, 2),
        "web_irrelevant_entity": ("verification_shortfall", 0, 2),
        "web_redirect": ("verified", 1, 1),
        "web_tavily_missing_credential": ("verification_shortfall", 0, 2),
        "web_tavily_429": ("verification_shortfall", 0, 2),
        "web_tavily_timeout": ("verification_shortfall", 0, 2),
        "web_tavily_malformed": ("verification_shortfall", 0, 2),
        "web_all_fetch_failure": ("verification_shortfall", 0, 2),
        "polish_accepted": ("verified", 3, 3),
        "polish_rejected": ("verified", 3, 3),
        "polish_unavailable": ("verified", 3, 3),
    }[case]
    actual = (outcome["state"], outcome["acquired"], outcome["requested"])
    if actual != expected:
        raise AssertionError(f"{case}: outcome={actual}, expected={expected}")

    completed = [
        event for event in events if event.get("type") == "logical_activity_completed"
    ]
    if case.startswith("mail_") or case.startswith("polish_"):
        reads = [event for event in completed if event.get("normalized_operation") == "mail.read"]
        if case in {"mail_denied", "mail_empty"}:
            if reads:
                raise AssertionError(f"{case}: body reads were not allowed")
        elif len(reads) != 3:
            raise AssertionError(f"{case}: expected three distinct read activities")
        if case == "mail_transient_retry":
            if [event["attempt_count"] for event in reads] != [2, 1, 1]:
                raise AssertionError(f"{case}: retry was not exactly once on the first read")

    if case.startswith("web_tavily_"):
        provider_events = [
            event for event in events
            if event.get("type") == "evidence_acquisition_diagnostic"
            and event.get("status") == "search_completed"
        ]
        providers = [event.get("provider") for event in provider_events]
        if providers != ["tavily", "duckduckgo"]:
            raise AssertionError(f"{case}: fallback providers={providers}")
    if case == "web_all_fetch_failure":
        fetches = [event for event in completed if event.get("normalized_operation") == "web.fetch"]
        if not 1 <= len(fetches) <= 5 or any(event.get("attempt_count", 0) > 2 for event in fetches):
            raise AssertionError(f"{case}: fetch failure was not bounded")
    if case == "web_redirect":
        text = "".join(event.get("content", "") for event in events if event.get("type") == "token")
        if "https://public-office.example/final" not in text or "/requested" in text:
            raise AssertionError(f"{case}: canonical citation did not use the validated final URL")
    expected_polish = {
        "polish_accepted": "accepted",
        "polish_rejected": "rejected",
        "polish_unavailable": "unavailable",
    }.get(case)
    if expected_polish:
        statuses = [event.get("status") for event in events if event.get("type") == "evidence_polish"]
        if statuses != [expected_polish]:
            raise AssertionError(f"{case}: polish status={statuses}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--token-file", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--swift-sse", type=pathlib.Path, required=True)
    args = parser.parse_args()
    token = args.token_file.read_text().strip()

    unauth_status, _ = request(
        args.base_url, None, "/acceptance/stage8/fixture", {"selection": None}
    )
    if unauth_status != 401:
        raise AssertionError(f"acceptance route unauthenticated status={unauth_status}, expected 401")
    auth_status, _ = request(
        args.base_url, token, "/acceptance/stage8/fixture", {"selection": None}
    )
    if auth_status != 200:
        raise AssertionError(f"acceptance route authenticated status={auth_status}, expected 200")

    campaigns = []
    all_payloads: list[str] = []
    for campaign in range(2):
        results = {}
        for name, acquisition, polish, prompt in CASES:
            status, _ = request(
                args.base_url,
                token,
                "/acceptance/stage8/fixture",
                {"selection": {"acquisition": acquisition, "polish": polish}},
            )
            if status != 200:
                raise AssertionError(f"{name}: fixture control status={status}")
            status, body = request(
                args.base_url,
                token,
                "/chat",
                {"message": prompt, "session_id": f"stage8-{campaign}-{name}"},
            )
            if status != 200:
                raise AssertionError(f"{name}: chat status={status}")
            events = sse_events(body)
            assert_terminal(name, events)
            assert_case_semantics(name, events)
            all_payloads.extend(json.dumps(event, separators=(",", ":")) for event in events)
            results[name] = structural_signature(events)
        campaigns.append(results)

    if campaigns[0] != campaigns[1]:
        differing = [name for name in campaigns[0] if campaigns[0][name] != campaigns[1][name]]
        raise AssertionError(f"campaigns were not structurally identical: {differing}")

    accepted = campaigns[0]["polish_accepted"]["token_sha256"]
    for name in ["polish_rejected", "polish_unavailable"]:
        if campaigns[0][name]["token_sha256"] != accepted:
            raise AssertionError(f"{name}: canonical bytes changed")

    request(args.base_url, token, "/acceptance/stage8/fixture", {"selection": None})
    args.output.write_text(json.dumps({
        "route": {"unauthenticated": unauth_status, "authenticated": auth_status},
        "campaigns_identical": True,
        "cases": campaigns[0],
    }, indent=2, sort_keys=True) + "\n")
    args.swift_sse.write_text("\n".join(all_payloads) + "\n")


if __name__ == "__main__":
    main()

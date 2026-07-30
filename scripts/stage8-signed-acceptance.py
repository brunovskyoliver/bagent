#!/usr/bin/env python3
"""Run every reproducible Stage 8 fixture through the signed Swift HTTP/SSE client."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile
import urllib.error
import urllib.request


CASES = [
    ("mail_complete", "mail_complete", "unavailable", "Can you read and summarize my 3 latest emails?"),
    ("mail_transient_retry", "mail_transient_retry", "unavailable", "Can you read and summarize my 3 latest emails?"),
    ("mail_partial", "mail_partial", "unavailable", "Can you read and summarize my 3 latest emails?"),
    ("mail_unavailable", "mail_unavailable", "unavailable", "Can you read and summarize my 3 latest emails?"),
    ("mail_denied", "mail_denied", "unavailable", "Can you read and summarize my 3 latest emails?"),
    ("mail_empty", "mail_empty", "unavailable", "Can you read and summarize my 3 latest emails?"),
    ("web_authoritative", "web_authoritative", "unavailable", "Who is the President of Slovakia? Use the official first-party website."),
    ("web_corroborated", "web_corroborated", "unavailable", "What is the current city proper population of Bratislava? Verify it with two independent publishers."),
    ("web_conflict", "web_conflict", "unavailable", "What is the current population of Bratislava? Verify it using two independent sources and show any conflict."),
    ("web_ambiguous_table", "web_ambiguous_table", "unavailable", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_irrelevant_entity", "web_irrelevant_entity", "unavailable", "Who is the current President of Slovakia? Verify it with two independent sources."),
    ("web_redirect", "web_redirect", "unavailable", "Who is the President of Slovakia? Use the official first-party website."),
    ("web_tavily_missing_credential", "web_tavily_missing_credential", "unavailable", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_tavily_429", "web_tavily_429", "unavailable", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_tavily_timeout", "web_tavily_timeout", "unavailable", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_tavily_malformed", "web_tavily_malformed", "unavailable", "What is the current population of Bratislava? Verify it using two independent sources."),
    ("web_ddg_fallback", "web_ddg_fallback", "unavailable", "Who is the President of Slovakia? Use the official first-party website."),
    ("web_all_fetch_failure", "web_all_fetch_failure", "unavailable", "What is the current capital of Slovakia? Verify it with two independent publishers."),
    ("polish_accepted", "mail_complete", "accepted", "Can you read and summarize my 3 latest emails?"),
    ("polish_rejected", "mail_complete", "rejected", "Can you read and summarize my 3 latest emails?"),
    ("polish_unavailable", "mail_complete", "unavailable", "Can you read and summarize my 3 latest emails?"),
]

# Reviewed canonical answer bytes and citation sets for every signed fixture.
# Keeping only hashes and byte counts makes the release proof exact without
# persisting Mail bodies, fixture passages, or raw SSE payloads.
EXPECTED_OUTPUTS = {
    "mail_complete": ("c8f21965b0a34f06fd38ccaa312f01b9d0383fc7d54a50ed97ed8499139eda31", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 308),
    "mail_denied": ("85099cc6a2241fa664e14a53d9671d49c5a1ee288d87bc1ff3ff910dc03ac9de", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 76),
    "mail_empty": ("e09eb37f97be2bc9ce203b4992d598b1cd06bbaaf3b4ad654a3e2644f394ccd1", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 107),
    "mail_partial": ("0ddcdfab9a841ffef27f430ad896bca686c7957d4d04337f41405744a0dbaa7d", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 158),
    "mail_transient_retry": ("c8f21965b0a34f06fd38ccaa312f01b9d0383fc7d54a50ed97ed8499139eda31", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 308),
    "mail_unavailable": ("0883bc983ba1ee1226dd19e6492a5f7a6f9ce0785458d951a4d13f1c21d5df51", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 103),
    "polish_accepted": ("c8f21965b0a34f06fd38ccaa312f01b9d0383fc7d54a50ed97ed8499139eda31", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 308),
    "polish_rejected": ("c8f21965b0a34f06fd38ccaa312f01b9d0383fc7d54a50ed97ed8499139eda31", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 308),
    "polish_unavailable": ("c8f21965b0a34f06fd38ccaa312f01b9d0383fc7d54a50ed97ed8499139eda31", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 308),
    "web_all_fetch_failure": ("8e490d2ff86e9872e8ddcd9bbd2e5366789409f0ba0078ae07af9c1ef90ce307", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 135),
    "web_ambiguous_table": ("a3bdb70c3acbdb0ff22095739e61266798e6d96026eba4b849bbe1cdab3a420b", "5f5afcb085819143fa14e799251b4f679bec2cb6a1bc663d6dedb87891e52132", 289),
    "web_authoritative": ("b374c10983370be9d2c7f142a69c4016d8b729b1a8c039d3459c4e13bfe731d6", "07d29843a3756a145e6f1a37dfa06d63ffd4fad035dda5cf01050c315b7289c4", 107),
    "web_conflict": ("27b3b3ad4eca2bc2c5d192021901c1fc111e31fbf220383dd8ba2ad1d26db0e0", "5f5afcb085819143fa14e799251b4f679bec2cb6a1bc663d6dedb87891e52132", 424),
    "web_corroborated": ("ab41279257520af6739e1509f5779e3f890742a3d6d954f431d79497988bf742", "5f5afcb085819143fa14e799251b4f679bec2cb6a1bc663d6dedb87891e52132", 226),
    "web_ddg_fallback": ("b374c10983370be9d2c7f142a69c4016d8b729b1a8c039d3459c4e13bfe731d6", "07d29843a3756a145e6f1a37dfa06d63ffd4fad035dda5cf01050c315b7289c4", 107),
    "web_irrelevant_entity": ("5fa6e9720c0b51d06c37f3dcd506eab1fb39ba75d1fab5b010d51fb2ef95bb26", "eca8e5dc204fe1f042f7d02e16eb19e7b0b7f7c609b3cd623a64fcba816a4e71", 225),
    "web_redirect": ("5313a50f6c1d642fedf2c1aefab415a5b81c98439689bafca995a12306d27cfa", "f5b6f0a6239fb29e6ba9b13b857e0c33521883a19c73c957a4e6e166c70fd3af", 103),
    "web_tavily_429": ("8e490d2ff86e9872e8ddcd9bbd2e5366789409f0ba0078ae07af9c1ef90ce307", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 135),
    "web_tavily_malformed": ("8e490d2ff86e9872e8ddcd9bbd2e5366789409f0ba0078ae07af9c1ef90ce307", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 135),
    "web_tavily_missing_credential": ("8e490d2ff86e9872e8ddcd9bbd2e5366789409f0ba0078ae07af9c1ef90ce307", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 135),
    "web_tavily_timeout": ("8e490d2ff86e9872e8ddcd9bbd2e5366789409f0ba0078ae07af9c1ef90ce307", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", 135),
}


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


def run_signed_case(
    signed_app: pathlib.Path,
    acquisition: str,
    polish: str,
    prompt: str,
    output: pathlib.Path,
) -> dict:
    environment = os.environ.copy()
    environment["BAGENT_STAGE8_ACCEPTANCE_FIXTURES"] = "1"
    subprocess.run(
        [
            str(signed_app),
            "--stage8-acceptance-case",
            acquisition,
            polish,
            prompt,
            str(output),
        ],
        check=True,
        env=environment,
        timeout=120,
    )
    return json.loads(output.read_text())


def assert_case_semantics(case: str, result: dict) -> None:
    if result["outcome_count"] != 1 or result["done_count"] != 1:
        raise AssertionError(f"{case}: expected exactly one outcome and one done")
    if not result["ui_outcome_present"]:
        raise AssertionError(f"{case}: signed UI presentation path did not retain the outcome")
    outcome = result["outcome"]
    actual_output = (
        result["token_sha256"],
        result["citation_set_sha256"],
        result["token_bytes"],
    )
    if actual_output != EXPECTED_OUTPUTS[case]:
        raise AssertionError(f"{case}: canonical answer or citation bytes changed")
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
        "web_ddg_fallback": ("verified", 1, 1),
        "web_all_fetch_failure": ("verification_shortfall", 0, 2),
        "polish_accepted": ("verified", 3, 3),
        "polish_rejected": ("verified", 3, 3),
        "polish_unavailable": ("verified", 3, 3),
    }[case]
    actual = (outcome["state"], outcome["acquired"], outcome["requested"])
    if actual != expected:
        raise AssertionError(f"{case}: outcome={actual}, expected={expected}")

    completed = result["activities"]
    if case.startswith("mail_") or case.startswith("polish_"):
        lists = [event for event in completed if event.get("operation") == "mail.list"]
        if len(lists) != 1:
            raise AssertionError(f"{case}: expected exactly one mail.list activity")
        reads = [event for event in completed if event.get("operation") == "mail.read"]
        if case in {"mail_denied", "mail_empty"}:
            if reads:
                raise AssertionError(f"{case}: body reads were not allowed")
        elif len(reads) != 3:
            raise AssertionError(f"{case}: expected three distinct read activities")
        elif len({event.get("argument_hash") for event in reads}) != 3:
            raise AssertionError(f"{case}: mail.read arguments were not three distinct opaque IDs")
        if case == "mail_transient_retry":
            if [event["attempt_count"] for event in reads] != [2, 1, 1]:
                raise AssertionError(f"{case}: retry was not exactly once on the first read")

    expected_providers = {
        "web_tavily_missing_credential": [
            ("tavily", "failed(connectorunavailable)"), ("duckduckgo", "empty")
        ],
        "web_tavily_429": [
            ("tavily", "failed(ratelimited)"), ("duckduckgo", "empty")
        ],
        "web_tavily_timeout": [("tavily", "timedout"), ("duckduckgo", "empty")],
        "web_tavily_malformed": [
            ("tavily", "invalidresponse"), ("duckduckgo", "empty")
        ],
        "web_ddg_fallback": [
            ("tavily", "failed(ratelimited)"),
            ("duckduckgo", "succeeded { result_count: 1 }")
        ],
    }.get(case)
    if expected_providers:
        providers = [
            (event.get("provider"), event.get("status")) for event in result["providers"]
        ]
        if providers != expected_providers:
            raise AssertionError(f"{case}: fallback providers={providers}")
        searches = [
            event for event in completed if event.get("operation") == "web.search"
        ]
        if len(searches) != 1 or searches[0].get("attempt_count") != 1:
            raise AssertionError(f"{case}: provider search was not bounded to one operation")
    if case == "web_all_fetch_failure":
        fetches = [event for event in completed if event.get("operation") == "web.fetch"]
        if not 1 <= len(fetches) <= 5 or any(event.get("attempt_count", 0) > 2 for event in fetches):
            raise AssertionError(f"{case}: fetch failure was not bounded")
    if case == "web_redirect":
        expected_hash = hashlib.sha256(
            b"https://public-office.example/final"
        ).hexdigest()
        if result["citation_set_sha256"] != expected_hash:
            raise AssertionError(f"{case}: canonical citation did not use the validated final URL")
    expected_polish = {
        "polish_accepted": "accepted",
        "polish_rejected": "rejected",
        "polish_unavailable": "unavailable",
    }.get(case)
    if expected_polish:
        statuses = result["polish_statuses"]
        if statuses != [expected_polish]:
            raise AssertionError(f"{case}: polish status={statuses}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--token-file", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--signed-app", type=pathlib.Path, required=True)
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
    with tempfile.TemporaryDirectory(prefix="bagent-stage8-signed-") as temp:
        temp_dir = pathlib.Path(temp)
        for campaign in range(2):
            results = {}
            for name, acquisition, polish, prompt in CASES:
                result = run_signed_case(
                    args.signed_app,
                    acquisition,
                    polish,
                    prompt,
                    temp_dir / f"{campaign}-{name}.json",
                )
                assert_case_semantics(name, result)
                results[name] = result
            campaigns.append(results)

    if campaigns[0] != campaigns[1]:
        differing = [name for name in campaigns[0] if campaigns[0][name] != campaigns[1][name]]
        raise AssertionError(f"campaigns were not structurally identical: {differing}")

    accepted = campaigns[0]["polish_accepted"]["token_sha256"]
    for name in ["polish_rejected", "polish_unavailable"]:
        if campaigns[0][name]["token_sha256"] != accepted:
            raise AssertionError(f"{name}: canonical bytes changed")

    request(args.base_url, token, "/acceptance/stage8/fixture", {"selection": None})
    repo_root = pathlib.Path(__file__).resolve().parents[1]
    source_commit = subprocess.run(
        ["git", "-C", str(repo_root), "rev-parse", "HEAD"],
        check=True,
        text=True,
        capture_output=True,
    ).stdout.strip()
    args.output.write_text(json.dumps({
        "provenance": {
            "source_commit": source_commit,
            "signed_app_sha256": hashlib.sha256(args.signed_app.read_bytes()).hexdigest(),
        },
        "route": {"unauthenticated": unauth_status, "authenticated": auth_status},
        "signed_swift_cases_per_campaign": len(CASES),
        "campaigns_identical": True,
        "cases": campaigns[0],
    }, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()

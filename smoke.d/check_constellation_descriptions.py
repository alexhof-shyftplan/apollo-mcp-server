#!/usr/bin/env python3
"""AIR-397 offline serve-smoke checker.

Speaks just enough MCP streamable HTTP to a locally booted apollo-mcp-server
running the constellation configuration (config/constellation/config.yaml) and
asserts, over the wire:

  * `execute` is the ONLY tool listed — as of AIR-402 (S2.5) the
    search/introspect/validate tools are served by the Discovery service
    (constellation-discovery) through the gateway's unified tool list, so
    this server must not list them,
  * the `execute` description carries the full access protocol (anticipate ->
    dry-run one batch -> one consolidated access request -> poll-for-approval
    via execute -> single bundle-digest-mismatch retry -> re-dry-run -> deploy),
  * denial grouping (one consolidated interaction per session),
  * the dry-run accuracy limit and the runtime-denial safety net,
  * the denial-error shape embedded in the descriptions equals the shared
    golden dry-run corpus (--golden) — the runtime's recorded output,
  * NO `constellation` skill is registered (tools, prompts if advertised).

Prints one "PASS <check>" / "FAIL <check> ..." line per check and a final
"RESULT: PASS|FAIL" line; exits non-zero on any failure.

Stdlib only — no pip dependencies.
"""

import argparse
import json
import sys
import urllib.request

PROTOCOL_VERSION = "2025-06-18"

# Phrase checks against whitespace-flattened description text. Mirrors the
# cargo fixture tests in crates/apollo-mcp-server/src/runtime/
# constellation_instructions.rs — keep the two lists in sync.
EXECUTE_PHRASES = [
    ("anticipate-operations", "Anticipate the app's operations"),
    ("dry-run-one-batch", "dry-run them in one batch"),
    ("dry-run-batch-signature", "dry_run(requests: [{scope, operation}, ...])"),
    ("dry-run-nothing-executes", "Nothing executes during a dry run"),
    ("dry-run-mutations-safe", "mutations are safe to include"),
    ("consolidated-request", "one consolidated access request"),
    ("denial-grouping-per-session", "one consolidated interaction per session"),
    ("denial-grouping-not-per-field", "Never submit one request per field or per operation"),
    ("poll-for-approval", "Poll for approval"),
    ("poll-via-execute", "status query through this `execute` tool"),
    ("re-dry-run-to-confirm", "re-run the SAME dry-run batch"),
    ("deploy", "Deploy the app only after the confirming dry run is clean"),
    ("digest-mismatch", "bundle-digest mismatch"),
    ("digest-mismatch-single-retry", "resubmit exactly ONCE"),
    ("digest-mismatch-no-second-retry", "do not retry a second time"),
    ("accuracy-limit", "point-in-time and advisory"),
    ("accuracy-indeterminate", "`indeterminate`"),
    ("runtime-safety-net", "runtime denial path remains the safety net"),
    ("handle-runtime-denials", "always handle runtime denial errors"),
]


def parse_body(raw, content_type):
    """Return the list of JSON-RPC messages in a response body (JSON or SSE)."""
    text = raw.decode("utf-8", "replace")
    if "text/event-stream" in (content_type or ""):
        messages = []
        for line in text.splitlines():
            if line.startswith("data:"):
                data = line[len("data:"):].strip()
                if data:
                    messages.append(json.loads(data))
        return messages
    text = text.strip()
    return [json.loads(text)] if text else []


class McpClient:
    def __init__(self, endpoint):
        self.endpoint = endpoint
        self.session_id = None
        self.next_id = 1

    def _post(self, payload):
        headers = {
            "content-type": "application/json",
            "accept": "application/json, text/event-stream",
        }
        if self.session_id:
            headers["mcp-session-id"] = self.session_id
            headers["mcp-protocol-version"] = PROTOCOL_VERSION
        req = urllib.request.Request(
            self.endpoint, data=json.dumps(payload).encode(), headers=headers, method="POST"
        )
        with urllib.request.urlopen(req, timeout=30) as resp:
            sid = resp.headers.get("mcp-session-id")
            if sid:
                self.session_id = sid
            return parse_body(resp.read(), resp.headers.get("content-type"))

    def request(self, method, params):
        req_id = self.next_id
        self.next_id += 1
        messages = self._post(
            {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
        )
        for message in messages:
            if message.get("id") == req_id:
                if "error" in message:
                    raise RuntimeError(f"{method} failed: {message['error']}")
                return message.get("result", {})
        raise RuntimeError(f"no response to {method} (got: {messages!r})")

    def initialize(self):
        result = self.request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "air397-constellation-smoke", "version": "0.0.1"},
            },
        )
        self._post({"jsonrpc": "2.0", "method": "notifications/initialized"})
        return result


def flat(text):
    """Collapse whitespace runs so phrase checks survive YAML line wrapping."""
    return " ".join(text.split())


def json_blocks(text):
    """Extract every fenced ```json block from description text."""
    blocks = []
    rest = text
    while True:
        start = rest.find("```json")
        if start < 0:
            return blocks
        rest = rest[start + len("```json"):]
        end = rest.find("```")
        if end < 0:
            raise ValueError("unterminated ```json block in description")
        blocks.append(json.loads(rest[:end]))
        rest = rest[end + 3:]


def golden_expected(golden, case_name):
    for case in golden["cases"]:
        if case["name"] == case_name:
            return case["expected"][0]
    raise KeyError(f"golden corpus case not found: {case_name}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", required=True, help="MCP endpoint, e.g. http://127.0.0.1:4597/mcp")
    parser.add_argument("--golden", required=True, help="path to the vendored dry-run-golden.json")
    args = parser.parse_args()

    with open(args.golden, encoding="utf-8") as f:
        golden = json.load(f)

    failures = 0

    def check(name, ok, detail=""):
        nonlocal failures
        if ok:
            print(f"PASS {name}")
        else:
            print(f"FAIL {name}{': ' + detail if detail else ''}")
            failures += 1

    client = McpClient(args.endpoint)
    init = client.initialize()

    # Server-level instructions are configuration-delivered guidance too.
    instructions = flat(init.get("instructions") or "")
    check(
        "init:instructions-carry-workflow",
        "one consolidated access request" in instructions and "dry-run" in instructions,
        f"instructions={instructions[:200]!r}",
    )

    tools = client.request("tools/list", {}).get("tools", [])
    by_name = {tool["name"]: tool.get("description", "") for tool in tools}
    print(f"tools registered: {sorted(by_name)}")

    # AIR-402 (S2.5): this server serves `execute` ONLY. The search/
    # introspect/validate tools moved to the Discovery service
    # (constellation-discovery) and reach agents through the gateway's
    # unified tool list, so they must NOT be listed here.
    check("tool-registered:execute", "execute" in by_name)
    for tool in ("introspect", "search", "validate"):
        check(
            f"tool-absent:{tool}",
            tool not in by_name,
            f"{tool} is served by constellation-discovery (AIR-402/S2.5)",
        )
    check(
        "tool-list:execute-only",
        set(by_name) == {"execute"},
        f"tools={sorted(by_name)}",
    )

    execute_desc = by_name.get("execute", "")
    execute_flat = flat(execute_desc)
    for key, phrase in EXECUTE_PHRASES:
        check(f"proto:{key}", phrase in execute_flat, f"missing phrase {phrase!r}")

    # Denial-error shape described over the wire == the runtime's recorded
    # output (the shared golden dry-run corpus).
    try:
        blocks = json_blocks(execute_desc)
    except ValueError as e:
        blocks = []
        check("denial-shape:blocks-parse", False, str(e))
    if blocks:
        check("denial-shape:blocks-parse", len(blocks) == 2, f"found {len(blocks)} blocks")
        decided = golden_expected(golden, "requestable-denial-mints-token")
        unresolvable = golden_expected(golden, "hidden-field-fails-resolution")
        check(
            "denial-shape:decided-matches-runtime",
            len(blocks) > 0 and blocks[0] == decided,
            "described decided-entry shape != golden runtime output",
        )
        check(
            "denial-shape:errors-matches-runtime",
            len(blocks) > 1 and blocks[1] == unresolvable,
            "described errors-only shape != golden runtime output",
        )
        token = decided["fields"][1]["denialContext"]
        check("denial-shape:token-verbatim", token in execute_desc)

    # No `constellation` skill registered anywhere the endpoint exposes.
    check(
        "no-skill:tools",
        not any("constellation" in name.lower() for name in by_name),
        f"tools={sorted(by_name)}",
    )
    capabilities = init.get("capabilities", {})
    if "prompts" in capabilities:
        try:
            prompts = client.request("prompts/list", {}).get("prompts", [])
            names = [prompt.get("name", "") for prompt in prompts]
            check(
                "no-skill:prompts",
                not any("constellation" in name.lower() for name in names),
                f"prompts={names}",
            )
        except RuntimeError as e:
            check("no-skill:prompts", False, f"prompts/list errored: {e}")
    else:
        check("no-skill:prompts", True, "")
    check("no-skill:capabilities", "constellation" not in json.dumps(capabilities).lower())

    print("RESULT: PASS" if failures == 0 else "RESULT: FAIL")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()

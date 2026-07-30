# Constellation access-workflow guidance (AIR-397 / S1.10)

`config.yaml` in this directory carries the agent guidance for working against a
Constellation-governed graph — the full access workflow (anticipate the app's
operations → dry-run them in one batch → submit one consolidated access request →
poll for approval via `execute` → re-dry-run to confirm → deploy), denial
grouping, the single retry on bundle-digest mismatch, and the dry-run accuracy
limit.

## Delivery: configuration, not a compiled skill

The guidance is **configuration-driven**. It reaches agents through the MCP
protocol itself:

- the `initialize` response `instructions` (the `instructions` key), and
- the tool descriptions returned by `tools/list` (the per-tool
  `introspection.<tool>.hint` keys, which the server appends to each built-in
  tool description).

The previous delivery vehicle — the `constellation` agent skill compiled into
the distribution bundle — is **deprecated and removed**. No skill named
`constellation` is registered in this repository (guarded by
`crates/apollo-mcp-server/src/runtime/constellation_instructions.rs`), and the
offline serve-smoke (`smoke.d/AIR-397.sh`) asserts none is registered over the
MCP endpoint. To change what agents are told, edit `config.yaml` and restart
the server — no rebuild, no skill recompilation.

## Usage

Layer the keys from `config.yaml` into the deployment's server config. The file
is a fragment: `transport`, `endpoint`, `schema`, and credentials remain
deployment-specific, and `APOLLO_MCP_*` environment variables still take
precedence over every key.

## Contract

The denial-error shapes quoted in the descriptions are pinned byte-for-byte
against the shared golden dry-run corpus
(`constellation-policy-eval/fixtures/dry-run-golden.json`, vendored at
`crates/apollo-mcp-server/src/runtime/testdata/dry-run-golden.json`) by the
fixture tests in `constellation_instructions.rs`. That corpus is itself pinned
byte-exact against the enforcement runtime by the eval crate's test suite, so
the shape agents are told is the shape the runtime actually emits.

Validation against **staging** fixtures (the deployed runtime's live output) is
a human-verified step — see `HANDOFF.md`.

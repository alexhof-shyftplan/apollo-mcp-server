# AIR-397 HANDOFF — skills → MCP tool descriptions

## What landed on `fable/AIR-397` (all green in-container)

- **Configuration-delivered guidance** — `config/constellation/config.yaml`:
  the full access workflow (anticipate the app's operations → dry-run them in
  one batch → one consolidated access request → poll for approval via
  `execute` → re-dry-run to confirm → deploy), denial grouping (one
  consolidated interaction per session), the single bundle-digest-mismatch
  retry (skew contract), and the dry-run accuracy limit + runtime-denial
  safety net — delivered through the MCP `initialize` `instructions` and the
  per-tool description `hint`s. No Rust changes were needed: the server's
  existing config plumbing carries everything, so the instructions are pure
  configuration, not compiled into any bundle.
- **Denial-shape fixture tests** —
  `crates/apollo-mcp-server/src/runtime/constellation_instructions.rs` (9
  tests, in `cargo test -p apollo-mcp-server`): the denial-error shape the
  descriptions describe is byte-equal to the shared golden dry-run corpus
  (`src/runtime/testdata/dry-run-golden.json`, vendored verbatim from
  `constellation-policy-eval/fixtures/dry-run-golden.json`; byte-parity with
  the sibling source is itself asserted when the checkout is present), plus
  protocol coverage and a no-`constellation`-skill guard.
- **Offline serve-smoke** — `smoke.d/AIR-397.sh` +
  `smoke.d/check_constellation_descriptions.py`: boots the real server with
  the new configuration (streamable HTTP, offline schema), lists tools over
  MCP, asserts the descriptions carry the full protocol, the described denial
  shape equals the golden corpus, and no `constellation` skill is registered.
  Result in-container: **31/31 checks passed** via
  `bash /work/harness/smoke.sh /work/AIR-397/apollo-mcp-server/smoke.d/AIR-397.sh`.
- Full `cargo test -p apollo-mcp-server --lib --bins`: 608 + 93 + 93 passed,
  0 failed. New files rustfmt-clean and clippy-clean.

## Human actions

1. **PR & merge** `fable/AIR-397` → `main`. Note the branch is cut on top of
   `fable/AIR-399` (search-quality baseline), so its PR includes those commits
   unless AIR-399 merges first — sequence the two PRs accordingly.
   Verify: CI green; `runtime::constellation_instructions` tests run in CI.
2. **Validate against STAGING fixtures** (the spec's human-verified portion):
   run a dry-run batch against the *deployed* staging runtime with a
   requestable-denial case and confirm the live denial output matches the
   shape the tool descriptions describe (`{requestId, denyOperation, fields:
   [{path, decision, reason?, denialContext?}]}` / `{requestId, errors}` —
   the vendored golden corpus). If staging ever diverges, fix the runtime or
   re-record the shared corpus in constellation-policy-eval first; the
   in-repo tests will then flag the description for update.
   Verify: staging dry-run output == the two JSON examples embedded in the
   `execute` tool description (modulo the entry-specific token/id bytes).
3. **Deprecate the published `constellation` skill artifact** wherever it
   lives outside this repo (skills registry / distribution bundle — e.g. the
   `apollographql/skills` source referenced by `skills-lock.json`). At the
   branch point, **no `constellation` skill existed anywhere in this
   repository** (verified by test + smoke, which also guard against
   reintroduction); the compiled artifact this story retires was never
   checked in here, so its removal from the external registry/bundle is a
   human step. Verify: the skills registry lists no `constellation` skill and
   agent sessions no longer load one.
4. **Wire the configuration into the deployment**: layer
   `config/constellation/config.yaml` into the deployed server's config
   (merge the `instructions` + `introspection.*.hint`/`enabled` keys, or
   concatenate the fragment as the smoke does — top-level keys are disjoint
   from deployment keys). Verify: `tools/list` on the deployed endpoint shows
   the workflow text (the smoke checker can be pointed at any endpoint:
   `python3 smoke.d/check_constellation_descriptions.py --endpoint <url>
   --golden crates/apollo-mcp-server/src/runtime/testdata/dry-run-golden.json`).
5. **Bare-name smoke invocation** (optional): `bash /work/harness/smoke.sh
   AIR-397` requires the profile inside the harness' read-only `smoke.d/`;
   copy or symlink `smoke.d/AIR-397.sh` there. The explicit-path invocation
   above works as-is.
6. **Deferred decision — where the dry_run/access-request operations
   surface**: the descriptions instruct agents to run `dry_run`, the
   access-request mutation, and its status query *through the `execute`
   tool* (per the S1.6 floor, those operations are always reachable). If the
   Front Door instead exposes them as dedicated MCP operation tools, add the
   per-operation guidance via `overrides.descriptions` in the same config
   file — the delivery mechanism is already in place.

---

# AIR-399 HANDOFF — search-quality baseline (carried on this branch)

## What landed on `fable/AIR-399` (all green in-container)

- **Offline catalog fixture** —
  `crates/apollo-mcp-server/src/introspection/tools/testdata/search_baseline/catalog.graphql`:
  a representative schema with service-prefixed domains (`Billing_`, `Inventory_`,
  `Shipping_`, `Support_`, `Accounts_`) plus unprefixed core types.
- **Captured baseline** — `.../search_baseline/baseline.json`: 14 queries → expected
  top-5 results (`top_paths` = ranked root paths from the index; `result_types` = type
  definitions the MCP `search` tool returns). Captured in-container by running today's
  search, not hand-authored. Coverage: 4 unscoped, 5 scoped, 5 known-hard
  (short natural-language vs prefixed names).
- **CI parity gate** — `.../tools/search_baseline.rs`:
  `baseline_reproduces_todays_search` and `baseline_covers_required_query_classes` run
  in the normal `cargo test` suite (green; verified deterministic across 4 consecutive
  runs). `capture_search_baseline` (`#[ignore]`) regenerates the fixture.
- **Offline serve-smoke** — `smoke.d/AIR-399.sh` + `smoke.d/check_search_baseline.py`:
  boots the real server (streamable HTTP, offline catalog, search tool only), replays
  all 14 baseline queries over MCP, asserts fixture reproduction.
  Result in-container: **16/16 checks passed** via
  `bash /work/harness/smoke.sh /work/AIR-399/apollo-mcp-server/smoke.d/AIR-399.sh`.
- Full `cargo test -p apollo-mcp-server --lib`: 608 passed, 0 failed.

## Human actions

1. **PR & merge** `fable/AIR-399` → `main`. The parity gate then runs in CI on every
   PR; any diff to `baseline.json` is a reviewable search-quality change.
   Verify: CI `cargo test` includes `search_baseline` tests and is green.
2. **Bare-name smoke invocation** (optional): `bash /work/harness/smoke.sh AIR-399`
   requires the profile inside the harness' read-only `smoke.d/`; copy or symlink
   `smoke.d/AIR-399.sh` there. The explicit-path invocation above works as-is.
3. **Deferred decision — production-catalog re-capture**: this baseline is captured
   over the checked-in offline catalog fixture (the only hermetic option in a
   credential-stripped container, and what CI can replay). If S2.5 parity should
   *additionally* be measured against the live production catalog, capture a second
   fixture set using `schema.source: uplink` with real `APOLLO_GRAPH_REF`/`APOLLO_KEY`;
   the test module and smoke checker need no code changes, only new fixture files.
   Verify: rerun the smoke against the new fixtures.
4. **Toolchain note** (no action strictly required): the container ran clippy 1.95,
   which flags pre-existing code (`apps/tool.rs`, `env_expansion.rs`,
   `introspection/tools/validate.rs`) under `--deny warnings`; the repo pins 1.92.0
   (`rust-toolchain.toml`) where CI is green. AIR-399 files are clippy-clean; the
   pre-existing lints were deliberately left untouched to keep the diff scoped.
5. **Container hygiene** (infra, for future runs): this story stalled ~5h on a full
   `/work` disk; stale `target/` caches of completed stories (e.g. `/work/AIR-389b`,
   14G) had to be cleared externally before the build could finish. Consider
   auto-cleaning story workspaces at run end.

## Notes for S2.5 (the consumer of this baseline)

- Discovery search must match or beat `top_paths`/`result_types` per query on this
  same catalog fixture; k = 5 = `MAX_SEARCH_RESULTS` (the search tool's truncation).
- Capture parameters are pinned in `baseline.json` under `captured_with` and asserted
  by the parity test, so silent config drift also fails the gate.

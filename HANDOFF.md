# AIR-399 HANDOFF — search-quality baseline

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

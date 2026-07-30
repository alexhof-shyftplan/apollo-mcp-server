# AIR-399 HANDOFF — search-quality baseline

## Status

All source artifacts for the search-quality baseline are implemented and on
`fable/AIR-399`:

- `crates/apollo-mcp-server/src/introspection/tools/testdata/search_baseline/catalog.graphql`
  — offline catalog fixture (service-prefixed domains + unprefixed core types).
- `crates/apollo-mcp-server/src/introspection/tools/search_baseline.rs` — capture +
  verify test module: `capture_search_baseline` (ignored; regenerates the fixture) and
  `baseline_reproduces_todays_search` / `baseline_covers_required_query_classes`
  (the CI parity gate, runs in the normal `cargo test` suite).
- `smoke.d/AIR-399.sh` + `smoke.d/check_search_baseline.py` — offline serve-smoke:
  boots the server (streamable HTTP, search tool only, offline catalog), replays every
  baseline query through the real MCP endpoint, asserts fixture reproduction.

## ⚠️ Pending: baseline.json capture (blocked in-container by disk exhaustion)

The container's 98G disk sat at **0 bytes free** for the entire run (≥27G is stale
`target/` caches from earlier completed stories: `/work/AIR-389b`, `/work/AIR-392`,
`/work/AIR-393`; two live stories then raced for the remaining blocks). The workspace
build ratcheted to ~1.4G of artifacts but `apollo-federation` could never complete, so
`baseline.json` could not be captured here. The fixture is intentionally absent rather
than hand-authored — hand-authoring would defeat the point of the baseline.

**To finish (any machine with ~3G free disk):**

```sh
git checkout fable/AIR-399
# 1. Capture the baseline from today's search (writes testdata/search_baseline/baseline.json)
cargo test -p apollo-mcp-server capture_search_baseline -- --ignored
# 2. Verify the parity gate is green and deterministic (run it a few times)
cargo test -p apollo-mcp-server search_baseline
cargo test -p apollo-mcp-server search_baseline
# 3. Offline serve-smoke: boot the real server and reproduce the fixtures end-to-end
bash /work/harness/smoke.sh /work/AIR-399/apollo-mcp-server/smoke.d/AIR-399.sh
# 4. Commit the captured fixture
git add crates/apollo-mcp-server/src/introspection/tools/testdata/search_baseline/baseline.json
git commit -m "AIR-399: captured search baseline fixture"
git push origin fable/AIR-399
```

Verify: step 2 passes (twice, proving determinism); step 3 prints
`PASS <query-id>` for all queries and `RESULT: PASS`.

## Human actions after the branch is green

1. **PR & merge** `fable/AIR-399` → main. The parity test then runs in CI on every PR;
   a diff to `baseline.json` is a reviewable search-quality change.
2. **Bare-name smoke invocation** (optional): `bash /work/harness/smoke.sh AIR-399`
   requires the profile inside the harness' read-only `smoke.d/`; copy or symlink
   `smoke.d/AIR-399.sh` there, or keep using the explicit-path invocation above.
3. **Container hygiene** (infra): remove stale caches of finished stories
   (`/work/AIR-389b/router-constellation/target` alone is 14G). This is what blocked
   the in-container capture.
4. **Deferred decision — production catalog re-capture**: this baseline is captured
   over the checked-in offline catalog fixture (the only option in a credential-stripped
   container, and the only way CI can replay it hermetically). If S2.5 parity should
   *additionally* be measured against the live production catalog, capture a second
   fixture set with `schema.source: uplink` + real `APOLLO_GRAPH_REF`/`APOLLO_KEY` and
   the same harness; the test module and smoke need no code changes, only fixtures.

## Design notes for S2.5 (the consumer)

- `baseline.json` pins per query: `top_paths` (ranked top-5 root paths from the index —
  the ranking the search tool selects from) and `result_types` (type definitions the MCP
  `search` tool returns — what a client observes). Discovery search must match or beat
  these on the same catalog fixture.
- k = 5 = `MAX_SEARCH_RESULTS` (the search tool's own truncation).
- Coverage classes are enforced by `baseline_covers_required_query_classes`:
  ≥3 each of `unscoped`, `scoped`, and `hard` (short natural-language queries against
  service-prefixed names, e.g. "track my package" → `Shipping_Parcel`).

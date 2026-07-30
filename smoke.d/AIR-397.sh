# AIR-397 offline serve-smoke profile (for the harness smoke.sh runner).
#
# Boots apollo-mcp-server with the NEW constellation configuration
# (config/constellation/config.yaml — the configuration-delivered replacement
# for the removed `constellation` skill) plus a minimal offline boot overlay,
# then, over the real MCP streamable HTTP endpoint, lists the tools and reads
# their descriptions, asserting:
#   - `execute` is the ONLY tool listed — as of AIR-402 (S2.5) the
#     search/introspect/validate tools are served by the Discovery service
#     (constellation-discovery) through the gateway's unified tool list,
#   - the full access protocol is carried by the execute description (dry-run
#     batch -> consolidated request -> poll-for-approval -> single
#     digest-mismatch retry -> re-dry-run -> deploy), denial grouping, and the
#     accuracy limit,
#   - the denial-error shape it describes equals the shared golden dry-run
#     corpus (the runtime's recorded output),
#   - no `constellation` skill is registered.
#
# Run:
#   bash /work/harness/smoke.sh /work/AIR-397/apollo-mcp-server/smoke.d/AIR-397.sh
# (or `bash /work/harness/smoke.sh AIR-397` when this profile is available in
# the harness smoke.d/ directory.)

air397_repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
air397_port="${AIR397_SMOKE_PORT:-4597}"
air397_golden="$air397_repo/crates/apollo-mcp-server/src/runtime/testdata/dry-run-golden.json"

smoke_build() {
  # debug=false + no incremental keeps the target dir small enough for constrained containers
  (cd "$air397_repo" && cargo build --config profile.dev.debug=false --config build.incremental=false -j 2 -p apollo-mcp-server --bin apollo-mcp-server)
}

smoke_start() {
  # The canonical constellation config is a fragment (instructions + tool
  # hints only); the boot overlay adds the deployment-specific keys. The two
  # are disjoint at the top level, so concatenation is a well-formed config.
  cat "$air397_repo/config/constellation/config.yaml" > "$WORK/air397-config.yaml"
  cat >> "$WORK/air397-config.yaml" <<EOF

# --- offline serve-smoke boot overlay (deployment-specific keys) ---
endpoint: http://127.0.0.1:1/
transport:
  type: streamable_http
  address: 127.0.0.1
  port: $air397_port
schema:
  source: local
  path: $air397_repo/smoke.d/constellation-smoke-schema.graphql
operations:
  source: local
  paths: []
health_check:
  enabled: true
EOF
  smoke_bg "$air397_repo/target/debug/apollo-mcp-server" "$WORK/air397-config.yaml"
}

smoke_ready() {
  http_ok "http://127.0.0.1:$air397_port/health"
}

smoke_check() {
  local out status
  out="$(python3 "$air397_repo/smoke.d/check_constellation_descriptions.py" \
    --endpoint "http://127.0.0.1:$air397_port/mcp" \
    --golden "$air397_golden" 2>&1)"
  status=$?
  printf '%s\n' "$out" | sed 's/^/    /'

  # One assertion per acceptance-criterion facet, so a regression names the
  # exact promise that broke.
  expect "instructions carry the workflow"        "$out" "PASS init:instructions-carry-workflow"
  expect "anticipate the app's operations"        "$out" "PASS proto:anticipate-operations"
  expect "dry-run in one batch"                   "$out" "PASS proto:dry-run-one-batch"
  expect "dry-run batch signature"                "$out" "PASS proto:dry-run-batch-signature"
  expect "dry run executes nothing"               "$out" "PASS proto:dry-run-nothing-executes"
  expect "mutations safe to dry-run"              "$out" "PASS proto:dry-run-mutations-safe"
  expect "one consolidated access request"        "$out" "PASS proto:consolidated-request"
  expect "denial grouping per session"            "$out" "PASS proto:denial-grouping-per-session"
  expect "never per-field requests"               "$out" "PASS proto:denial-grouping-not-per-field"
  expect "poll for approval"                      "$out" "PASS proto:poll-for-approval"
  expect "polling goes via execute"               "$out" "PASS proto:poll-via-execute"
  expect "re-dry-run to confirm"                  "$out" "PASS proto:re-dry-run-to-confirm"
  expect "deploy after clean confirm"             "$out" "PASS proto:deploy"
  expect "digest-mismatch covered"                "$out" "PASS proto:digest-mismatch"
  expect "digest-mismatch single retry"           "$out" "PASS proto:digest-mismatch-single-retry"
  expect "no second digest retry"                 "$out" "PASS proto:digest-mismatch-no-second-retry"
  expect "dry-run accuracy limit"                 "$out" "PASS proto:accuracy-limit"
  expect "indeterminate decisions stated"         "$out" "PASS proto:accuracy-indeterminate"
  expect "runtime denial safety net"              "$out" "PASS proto:runtime-safety-net"
  expect "agents told to handle runtime denials"  "$out" "PASS proto:handle-runtime-denials"
  expect "execute tool listed"                    "$out" "PASS tool-registered:execute"
  expect "introspect absent (Discovery serves it)" "$out" "PASS tool-absent:introspect"
  expect "search absent (Discovery serves it)"    "$out" "PASS tool-absent:search"
  expect "validate absent (Discovery serves it)"  "$out" "PASS tool-absent:validate"
  expect "execute is the only listed tool"        "$out" "PASS tool-list:execute-only"
  expect "described denial shape == runtime out"  "$out" "PASS denial-shape:decided-matches-runtime"
  expect "described error shape == runtime out"   "$out" "PASS denial-shape:errors-matches-runtime"
  expect "denialContext token verbatim"           "$out" "PASS denial-shape:token-verbatim"
  expect "no constellation skill in tools"        "$out" "PASS no-skill:tools"
  expect "no constellation skill in prompts"      "$out" "PASS no-skill:prompts"
  expect "no constellation skill in capabilities" "$out" "PASS no-skill:capabilities"
  expect "checker verdict"                        "$out" "RESULT: PASS"
  if [ "$status" -eq 0 ]; then ok "checker exited 0"; else bad "checker exited $status"; fi
}

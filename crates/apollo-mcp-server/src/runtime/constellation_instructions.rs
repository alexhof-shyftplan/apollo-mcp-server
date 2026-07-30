//! AIR-397 (S1.10) — Constellation access-workflow guidance is delivered via
//! configuration, not a compiled skill.
//!
//! The guidance lives in `config/constellation/config.yaml` (repo root) and
//! reaches agents through the MCP protocol itself: the `initialize` response
//! `instructions` plus the per-tool description `hint`s the server already
//! supports. These tests pin the artifact:
//!
//! * it parses as a valid server [`Config`] (typo'd keys fail here, not at boot),
//! * `execute` is the ONLY tool this server enables in the Constellation
//!   deployment — as of AIR-402 (S2.5) the `search`/`introspect`/`validate`
//!   tools are served by the Discovery service (constellation-discovery)
//!   through the gateway's unified tool list, so they are disabled here,
//! * the `execute` description covers the full access protocol (anticipate →
//!   dry-run one batch → one consolidated access request → poll for approval
//!   via `execute` → re-dry-run → deploy), denial grouping, the single
//!   bundle-digest-mismatch retry, and the dry-run accuracy limit,
//! * the denial-error shape the descriptions describe is byte-equal to the
//!   runtime's actual output, pinned by the shared golden dry-run corpus
//!   (`testdata/dry-run-golden.json`, vendored verbatim from
//!   `constellation-policy-eval/fixtures/dry-run-golden.json` — the corpus the
//!   eval crate proves byte-exact against the enforcement runtime),
//! * no `constellation` skill remains registered anywhere in this repository.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::Config;

const CONFIG_PATH: &str = "config/constellation/config.yaml";
const GOLDEN: &str = include_str!("testdata/dry-run-golden.json");

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn config_source() -> String {
    let path = repo_root().join(CONFIG_PATH);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn config() -> Config {
    serde_yaml::from_str(&config_source())
        .expect("config/constellation/config.yaml must parse as a valid server Config")
}

fn execute_hint(config: &Config) -> String {
    config
        .introspection
        .execute
        .hint
        .clone()
        .expect("execute tool hint must be configured")
}

/// Collapse whitespace runs to single spaces so phrase assertions are
/// insensitive to YAML line wrapping.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract every fenced ```json block from guidance text.
fn json_blocks(text: &str) -> Vec<Value> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```json") {
        let after = &rest[start + "```json".len()..];
        let end = after.find("```").expect("unterminated ```json block");
        blocks.push(
            serde_json::from_str(&after[..end]).expect("embedded JSON example must parse as JSON"),
        );
        rest = &after[end + 3..];
    }
    blocks
}

/// The expected runtime output of a named golden-corpus case (first batch entry).
fn golden_expected(case_name: &str) -> Value {
    let golden: Value = serde_json::from_str(GOLDEN).expect("golden corpus parses");
    golden["cases"]
        .as_array()
        .expect("golden corpus has cases")
        .iter()
        .find(|case| case["name"] == case_name)
        .unwrap_or_else(|| panic!("golden corpus case not found: {case_name}"))["expected"][0]
        .clone()
}

#[test]
fn config_is_valid_and_execute_carries_the_guidance() {
    let config = config();
    assert!(
        config.instructions.is_some(),
        "server-level instructions must be configured"
    );
    assert!(
        config.introspection.execute.hint.is_some(),
        "execute tool must carry a description hint"
    );
    assert!(
        config.introspection.execute.enabled,
        "execute tool must be enabled by this configuration"
    );
}

/// AIR-402 (S2.5): in the Constellation deployment, `execute` is the ONLY
/// tool this server enables. The `search`/`introspect`/`validate` tools are
/// served by the Discovery service (constellation-discovery) through the
/// gateway's unified tool list, so this config must disable them (and no
/// longer carry hints for tools it does not serve).
#[test]
fn execute_is_the_only_enabled_tool_in_the_constellation_config() {
    let config = config();
    assert!(
        config.introspection.execute.enabled,
        "execute must stay enabled: it is the tool this server still serves"
    );
    for (tool, enabled, hint) in [
        (
            "introspect",
            config.introspection.introspect.enabled,
            &config.introspection.introspect.hint,
        ),
        (
            "search",
            config.introspection.search.enabled,
            &config.introspection.search.hint,
        ),
        (
            "validate",
            config.introspection.validate.enabled,
            &config.introspection.validate.hint,
        ),
    ] {
        assert!(
            !enabled,
            "{tool} must be disabled: it is served by constellation-discovery (AIR-402/S2.5)"
        );
        assert!(
            hint.is_none(),
            "{tool} must not carry a hint: this server no longer serves it"
        );
    }
}

#[test]
fn descriptions_cover_the_full_access_protocol() {
    let config = config();
    let hint = flat(&execute_hint(&config));
    for step in [
        "Anticipate the app's operations",
        "dry-run them in one batch",
        "dry_run(requests: [{scope, operation}, ...])",
        "Nothing executes during a dry run",
        "mutations are safe to include",
        "one consolidated access request",
        "Poll for approval",
        "through this `execute` tool",
        "re-run the SAME dry-run batch",
        "Deploy",
    ] {
        assert!(
            hint.contains(step),
            "execute description must cover protocol step: {step:?}"
        );
    }
    // The workflow summary is also server-level guidance. (As of AIR-402/S2.5
    // the search/introspect/validate descriptions that routed back here are
    // served by constellation-discovery, not by this config.)
    let instructions = flat(&config.instructions.clone().unwrap_or_default());
    assert!(instructions.contains("dry-run"));
    assert!(instructions.contains("one consolidated access request"));
}

#[test]
fn denial_grouping_is_one_consolidated_interaction_per_session() {
    let hint = flat(&execute_hint(&config()));
    assert!(hint.contains("one consolidated interaction per session"));
    assert!(hint.contains("Never submit one request per field or per operation"));
    assert!(
        hint.contains("across the entire batch"),
        "grouping must span the whole dry-run batch"
    );
}

#[test]
fn poll_for_approval_goes_through_execute() {
    let hint = flat(&execute_hint(&config()));
    assert!(hint.contains("Poll for approval"));
    assert!(hint.contains("status query through this `execute` tool"));
}

#[test]
fn digest_mismatch_retry_is_exactly_one() {
    let hint = flat(&execute_hint(&config()));
    assert!(hint.contains("bundle-digest mismatch"));
    assert!(hint.contains("resubmit exactly ONCE"));
    assert!(hint.contains("do not retry a second time"));
}

#[test]
fn accuracy_limit_and_runtime_safety_net_are_stated() {
    let hint = flat(&execute_hint(&config()));
    assert!(hint.contains("point-in-time and advisory"));
    assert!(hint.contains("`indeterminate`"));
    assert!(hint.contains("runtime denial path remains the safety net"));
    assert!(hint.contains("always handle runtime denial errors"));
}

/// The denial-error shape the descriptions describe must equal the runtime's
/// actual output, verbatim. The embedded examples are compared against the
/// shared golden dry-run corpus — the recorded, byte-pinned runtime output for
/// exactly the inputs the description quotes.
#[test]
fn described_denial_shape_is_the_runtime_output_verbatim() {
    let hint = execute_hint(&config());
    let blocks = json_blocks(&hint);
    assert_eq!(
        blocks.len(),
        2,
        "execute description must embed the decided-entry and errors-only examples"
    );

    let decided = golden_expected("requestable-denial-mints-token");
    assert_eq!(
        blocks[0], decided,
        "described denial shape must match the runtime's output for `{{ contact {{ payroll }} }}`"
    );

    let unresolvable = golden_expected("hidden-field-fails-resolution");
    assert_eq!(
        blocks[1], unresolvable,
        "described resolution-failure shape must match the runtime's output"
    );

    // Verbatim guard on the opaque strings the agent will handle: the exact
    // token and requestId bytes from the runtime output appear in the text.
    let token = decided["fields"][1]["denialContext"]
        .as_str()
        .expect("golden case carries a denialContext token");
    assert!(token.starts_with("v0."));
    assert!(
        hint.contains(token),
        "denialContext token must appear verbatim"
    );

    let request_id = decided["requestId"].as_str().expect("requestId present");
    assert_eq!(request_id.len(), 26, "requestId is a 26-char ULID");
    assert!(
        request_id
            .chars()
            .all(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c)),
        "requestId is Crockford base32 uppercase"
    );
    assert!(hint.contains(request_id));
}

/// The vendored corpus must stay byte-identical to the shared contract in
/// constellation-policy-eval. The sibling checkout exists in the story
/// workspace (and any dev setup that clones the two repos side by side);
/// elsewhere the vendored copy is the pinned source and this check is skipped.
#[test]
fn vendored_golden_corpus_matches_shared_contract_source() {
    let source = repo_root().join("../constellation-policy-eval/fixtures/dry-run-golden.json");
    let Ok(source_bytes) = fs::read(&source) else {
        eprintln!(
            "skipping: shared-contract source not present at {}",
            source.display()
        );
        return;
    };
    assert_eq!(
        String::from_utf8_lossy(&source_bytes),
        GOLDEN,
        "vendored dry-run-golden.json must be byte-identical to the constellation-policy-eval fixture"
    );
}

/// AIR-397 removes the `constellation` skill: the guidance is configuration
/// now. Guard every skill registry in the repository against reintroduction.
#[test]
fn no_constellation_skill_in_repository() {
    let root = repo_root();

    let lock = fs::read_to_string(root.join("skills-lock.json"))
        .expect("skills-lock.json exists at the repo root");
    assert!(
        !lock.to_lowercase().contains("constellation"),
        "skills-lock.json must not register a constellation skill"
    );

    for skills_dir in [root.join(".claude/skills"), root.join(".agents/skills")] {
        let Ok(entries) = fs::read_dir(&skills_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            assert!(
                !name.contains("constellation"),
                "{} must not contain a constellation skill (found {name})",
                skills_dir.display()
            );
        }
    }
}

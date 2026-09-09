use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::Deserialize;

/// Progressive tool disclosure — bootstrap set + named tiers.
///
/// When either `bootstrap` or `tiers` is set, the server serves a
/// per-session filtered `tools/list`: on connect the session sees the
/// bootstrap set (plus the synthetic `load_tier` meta-tool). The client
/// then calls `load_tier(tier: "…")` to unlock a domain's tools
/// mid-session; the server emits `notifications/tools/list_changed`
/// and the client re-fetches, now seeing the expanded set.
///
/// If both `bootstrap` and `tiers` are empty, the server behaves
/// exactly as before: every operation and native tool is always
/// visible, and the synthetic `load_tier` tool is NOT exposed.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Tools {
    /// Tool names always visible at session start, before any
    /// `load_tier` call. Include tools that seed the session:
    /// authentication, session briefing, and cross-cutting reads that
    /// every workflow needs.
    pub bootstrap: Vec<String>,

    /// Named tool sets. A session must call
    /// `load_tier(tier: "<name>")` before the tools listed under
    /// that tier become visible in `tools/list`.
    ///
    /// Tier names are opaque strings; the LLM sees them via the
    /// `load_tier` tool description (built from the map keys) and
    /// picks one based on the user's intent.
    pub tiers: HashMap<String, Vec<String>>,
}

impl Tools {
    /// True when progressive disclosure is configured.
    pub fn is_enabled(&self) -> bool {
        !self.bootstrap.is_empty() || !self.tiers.is_empty()
    }

    /// Every tool name mentioned in either `bootstrap` or any tier.
    /// Used at startup to validate that every configured name matches
    /// a real tool (operation or native).
    pub fn referenced_tool_names(&self) -> HashSet<&str> {
        let mut out: HashSet<&str> = self.bootstrap.iter().map(String::as_str).collect();
        for tools in self.tiers.values() {
            out.extend(tools.iter().map(String::as_str));
        }
        out
    }
}

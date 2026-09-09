//! Progressive tool disclosure.
//!
//! When [`crate::runtime::tools::Tools`] is configured with a
//! `bootstrap` set and/or `tiers`, the server exposes a slimmed
//! `tools/list` per session: the bootstrap set is always visible,
//! and additional tiers become visible only after the client calls
//! the synthetic `load_tier` meta-tool.
//!
//! The state machine is per-session and lives entirely on the
//! `Running` state. This module carries the [`LoadTier`] tool
//! implementation plus the session-state helpers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, Tool};
use rmcp::schemars::JsonSchema;
use rmcp::serde_json::{Value, json};
use rmcp::{schemars, serde_json};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::errors::McpError;
use crate::schema_from_type;

/// The name of the synthetic tool that unlocks tiers.
pub const LOAD_TIER_TOOL_NAME: &str = "load_tier";

/// Session identifier used to key per-session state. Populated from
/// the `Mcp-Session-Id` header on HTTP transports; stdio callers
/// share the sentinel [`STDIO_SESSION_ID`].
pub type SessionId = String;

/// Sentinel session id used when the transport does not surface one
/// (stdio, unit tests). All stdio traffic shares this key.
pub const STDIO_SESSION_ID: &str = "__stdio__";

/// Per-session tier state.
#[derive(Debug, Default, Clone)]
pub struct SessionTiers {
    /// Tier names the LLM has already unlocked in this session.
    pub unlocked: HashSet<String>,
}

/// Shared map of per-session tier state. Cheap to clone (Arc).
pub type SessionStateMap = Arc<RwLock<HashMap<SessionId, SessionTiers>>>;

/// Input for the `load_tier` synthetic tool.
#[derive(JsonSchema, Deserialize, Debug)]
pub struct Input {
    /// The tier to unlock. Must match one of the tier names the
    /// server was configured with. See the tool description for the
    /// available tiers.
    pub tier: String,
}

/// The synthetic `load_tier` tool. Not backed by a `.graphql`
/// operation; the tool description is composed from the configured
/// tier names at startup.
#[derive(Clone, Debug)]
pub struct LoadTier {
    /// Config: tier name → tool names.
    tier_definitions: Arc<HashMap<String, Vec<String>>>,
    /// Reverse index: tool name → tier name. Built once at
    /// construction so `list_tools_impl` can look up the tier for a
    /// tool without scanning every tier per call.
    tool_to_tier: Arc<HashMap<String, String>>,
    /// Shared per-session unlocked-tier state.
    session_state: SessionStateMap,
    /// The MCP tool descriptor advertised on `tools/list`.
    pub tool: Tool,
}

impl LoadTier {
    pub fn new(
        tier_definitions: HashMap<String, Vec<String>>,
        session_state: SessionStateMap,
    ) -> Self {
        let mut tool_to_tier: HashMap<String, String> = HashMap::new();
        for (tier, tools) in &tier_definitions {
            for tool_name in tools {
                tool_to_tier.insert(tool_name.clone(), tier.clone());
            }
        }
        let tool = Tool::new(
            LOAD_TIER_TOOL_NAME,
            build_description(&tier_definitions),
            schema_from_type!(Input),
        );
        Self {
            tier_definitions: Arc::new(tier_definitions),
            tool_to_tier: Arc::new(tool_to_tier),
            session_state,
            tool,
        }
    }

    /// The tier that owns `tool_name`, if any. Returns `None` for
    /// bootstrap tools, native tools, and unknown names.
    pub fn tier_for_tool(&self, tool_name: &str) -> Option<&str> {
        self.tool_to_tier.get(tool_name).map(String::as_str)
    }

    /// Mark `tier` as unlocked in the given session. Returns the list
    /// of tool names now available under that tier so the LLM can see
    /// what it just gained before it re-fetches `tools/list`.
    #[tracing::instrument(skip(self))]
    pub async fn execute(
        &self,
        input: Input,
        session_id: SessionId,
    ) -> Result<CallToolResult, McpError> {
        let tier_name = input.tier;
        let Some(tools) = self.tier_definitions.get(&tier_name) else {
            let available: Vec<&str> =
                self.tier_definitions.keys().map(String::as_str).collect();
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Unknown tier {tier_name:?}. Available tiers: {available:?}"
            ))]));
        };

        let mut state = self.session_state.write().await;
        let entry = state.entry(session_id.clone()).or_default();
        let already_unlocked = !entry.unlocked.insert(tier_name.clone());

        let payload = json!({
            "tier": &tier_name,
            "already_unlocked": already_unlocked,
            "tools_now_available": tools,
            "unlocked_tiers": entry.unlocked.iter().collect::<Vec<_>>(),
        });

        // Also emit a human-readable one-liner so hosts that only
        // render text see something meaningful in the transcript.
        // Every tier tool was already visible in `tools/list` with a
        // slim placeholder description; the LLM's job is to refresh
        // its palette (auto on `list_changed`-aware hosts, via
        // `tool_search` on Claude Desktop) so the full descriptions
        // and input schemas replace the placeholders.
        let message = if already_unlocked {
            format!("Tier {tier_name:?} was already unlocked in this session.")
        } else {
            format!(
                "Unlocked tier {tier_name:?} for this session. {n} tool(s) now carry \
                 their full descriptions and input schemas: {tools:?}. IMPORTANT next \
                 step: refresh your tool palette so the swap propagates — on Claude \
                 Desktop that means running `tool_search(\"{tier_name}\")`; hosts that \
                 auto-refresh on tools/list_changed can skip it. Then invoke the tool \
                 the user's request actually needs.",
                n = tools.len(),
                tools = tools,
                tier_name = tier_name,
            )
        };

        Ok(CallToolResult::success(vec![
            ContentBlock::text(message),
            ContentBlock::text(serde_json::to_string(&payload).unwrap_or_default()),
        ]))
    }

    /// The set of tool names visible to `session_id` because the
    /// session unlocked one or more tiers that list them.
    pub async fn unlocked_tool_names_for_session(&self, session_id: &str) -> HashSet<String> {
        let state = self.session_state.read().await;
        let Some(session) = state.get(session_id) else {
            return HashSet::new();
        };
        let mut out = HashSet::new();
        for tier in &session.unlocked {
            if let Some(tools) = self.tier_definitions.get(tier) {
                out.extend(tools.iter().cloned());
            }
        }
        out
    }

    /// Slim placeholder description shown for a tool whose tier the
    /// current session has not unlocked yet. Keeps `tools/list` lean
    /// on cold start while telling the LLM exactly how to expand the
    /// full description. The tool's `inputSchema` is NOT slimmed —
    /// it stays full so hosts that snapshot the tool palette at
    /// connect (Claude Desktop) can still invoke the tool with
    /// meaningful arguments before `load_tier` is called.
    pub fn slim_description_for_locked_tier(tier: &str, tool_name: &str) -> String {
        format!(
            "`{tool_name}` is one of the tools in the `{tier}` domain. Its arguments \
             are already documented in this schema, but the full description of what \
             it does, when to use it, and how it composes with sibling tools is only \
             expanded after the session calls `load_tier(tier: \"{tier}\")`. Prefer \
             calling `load_tier` first when the user's request implies this domain, \
             so you can compare tools before invoking one."
        )
    }
}

fn build_description(tier_definitions: &HashMap<String, Vec<String>>) -> String {
    if tier_definitions.is_empty() {
        return "Unlock a tier of tools mid-session. No tiers are configured on this server."
            .to_string();
    }

    let mut tier_names: Vec<&String> = tier_definitions.keys().collect();
    tier_names.sort();
    let mut out = String::from(
        "Domain tools always appear in tools/list, but the ones outside the \
         session's currently-unlocked tiers are advertised with a slim placeholder \
         description and empty input schema until you call this tool. Call \
         `load_tier(tier: \"<name>\")` the moment the user's request implies a \
         domain — do not ask the user first, do not narrate the step. Available \
         tiers:\n",
    );
    for tier in tier_names {
        let count = tier_definitions.get(tier).map(Vec::len).unwrap_or(0);
        out.push_str(&format!("- `{tier}` ({count} tools)\n"));
    }
    out.push_str(
        "\nThe response you receive from this tool includes an explicit next-step \
         instruction; follow it exactly (typically it tells you to refresh your \
         palette via `tool_search` on Claude Desktop so the newly-expanded \
         descriptions and schemas are visible to the model).\n\n\
         Unlocked tiers stay unlocked for the rest of the session. Loading a tier \
         twice is a safe no-op.",
    );
    out
}

/// Extract the MCP session id from HTTP request extensions, falling
/// back to the stdio sentinel when the header is absent.
pub fn session_id_from_extensions(extensions: &rmcp::model::Extensions) -> SessionId {
    extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.headers.get("mcp-session-id"))
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| STDIO_SESSION_ID.to_string())
}

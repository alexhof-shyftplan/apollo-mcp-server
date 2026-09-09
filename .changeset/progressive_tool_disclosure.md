---
default: minor
---

# Progressive tool disclosure — `load_tier` meta-tool + `tools/list_changed`

Large MCP servers (dozens of operations, verbose descriptions) burn a
significant share of the session context budget the moment the client
runs `tools/list`. Every tool the LLM will never touch during this
particular workflow is still shipped over the wire, weighed by the
model, and paid for.

This changeset lets a config author split the tool catalogue into a
small **bootstrap** set plus one or more named **tiers**, and expose
a synthetic `load_tier` meta-tool that the LLM calls to unlock the
tier that matches the user's intent. Session state is server-side;
the client experiences it as an ordinary
`notifications/tools/list_changed` broadcast followed by its own
re-fetch of `tools/list`.

## Configuration

```yaml
tools:
  bootstrap:
    - WhoAmI
    - GetCompanySettingsCatalog
  tiers:
    scheduling:
      - ListShifts
      - ListShiftSchedules
      - CreateShiftAssignment
    absence:
      - ListAbsences
      - CreateAbsence
      - UpdateAbsence
```

When both `bootstrap` and `tiers` are empty, the server behaves
exactly as before — all tools are always visible and `load_tier` is
not exposed.

Startup fails with a clear error when a name in `tools.bootstrap` or
`tools.tiers` does not resolve to a known operation or native tool.
`load_tier` is reserved and cannot appear in either list.

## Behaviour

- On session start, `tools/list` returns only the bootstrap set plus
  the synthetic `load_tier` tool.
- Each `load_tier(tier: "…")` call adds the named tier's tools to
  the session's visibility set and emits
  `notifications/tools/list_changed` to every connected peer. Each
  session's next `tools/list` re-fetch reflects only that session's
  own unlocked tiers, so peers whose state did not change re-fetch
  the same list they already had.
- Loading a tier a second time is a safe no-op.
- Unlocked tiers persist for the life of the session. Session state
  is keyed on the `Mcp-Session-Id` HTTP header; stdio traffic is
  treated as a single ambient session.
- Progressive disclosure has no effect on operations invoked
  directly by tool name — the server does not enforce a "tier
  membership" gate on `tools/call`. This keeps the mechanism a
  discovery aid rather than an authorization boundary.

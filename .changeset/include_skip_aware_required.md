---
default: patch
---

# Skip `@include` and `@skip`-gated fields when computing `outputSchema.required`

`outputSchema` marked every non-null field in a tool's selection set as
`required` unconditionally. When an operation used `@include(if: …)` or
`@skip(if: …)` on a non-null field, the GraphQL response legitimately
omitted the field whenever the condition excluded it, and client-side
JSON Schema validators (observed on Claude Desktop's `ajv` integration)
rejected the payload as schema-invalid, surfacing only a generic
"Tool execution failed" with no cause upstream.

The walker now inspects `@include` and `@skip` directives on each
selection and drops the field from the emitted `required[]` when either
directive is present. The field's type stays non-null; only the
`required` bit becomes conditional, which matches the GraphQL execution
semantics.

/**
 * Re-export of the wire-side ACP `ToolKind` so consumers reading
 * either `@constants/ui` or `@constants/wire/formatted-tool-call`
 * see the same enum identity. The closed-set ACP-spec kinds drive
 * the frontend's `(kind, adapter, wireName) → Presentation` lookup.
 */
export { ToolKind } from '@constants/wire/formatted-tool-call'

# Toolbar layout service

A portable plugin can contribute a generic layout service using
`ctx.services.register({id, title, metadata: {surface: 'toolbar-layout', targets: ['capture.selection']}}, handler)`.
It requires `ui.render`. The host invokes it with
`{type: 'layout', surface: 'capture.selection', items: [{id, title, kind}, ...]}`.
`kind` is `button` or `separator`; never infer it from ID prefixes or labels.
Return `{order: ['copy', 'draw', ...]}`. IDs are unique strings; at most 128.
Unknown IDs are ignored, omitted items are appended in the host's normal order.
The plugin cannot remove/disable buttons or change their handlers. The DOM order
and keyboard navigation follow the returned order. Invalid responses restore the
normal order. Disabling/uninstalling the plugin also restores normal order.

Available selection toolbar IDs are `reference`, `add-region`, `delete-region`,
`draw`, `workflow`, `extensions`, `ocr`, `translate`, `scroll`, `record`, `copy`,
`export`, `pin`, and separators `separator-edit`, `separator-capture`,
`separator-export`. Availability depends on enabled capabilities. Always retain
unknown/new IDs when editing a saved preference. Layout services are composed in
stable plugin-ID/service-ID order; a later service can refine the previous order.

For the settings editor call `await ctx.ui.toolbars.describe(surface)` (requires
`ui.render`, capability `ui.toolbar-catalog`). It returns `{version, surface,
items, current, currentScope}`. `items` contains the complete host definition,
localized captions and explicit separator kinds. `current` is the most recent
rendered context, or null before first use, with any contextual captions.
Use definition IDs/kinds, prefer matching current labels, and retain new IDs.
Contextual definitions are not a promise that the button is always visible.
Do not invent captions such as "OCR 识别"/"钉住" when the host says "文字"/"贴图".
Display separator rows and allow ordering them with the other items. Do not
silently remove separator IDs from saved preferences or infer them from names.

The plugin owns its settings UI and persistence. Register a UI service with
`metadata: {surface: 'settings'}` and return `{html,width,height}` for its
`input.type === 'mount'`. The host injects the global `mewu` client into that HTML;
await `mewu.ready` then `mewu.call(input)` to invoke this same settings handler.
Use `ctx.configuration.get('order')` from both services.
From the live settings surface, use
`ctx.configuration.compareAndSet('order', previousValue, nextValue)` and handle
a false result by reloading. Configuration changes invalidate active layouts.
Never write user preferences on activation. Normal layout services are pure
queries: do not call native actions, show dialogs or request extra permissions.
Each service call uses a fresh interpreter: module globals are not shared
between layout and settings invocations. Re-read the catalog and configuration
for each call. Do not store observations as a substitute for the host catalog.

The host resolves initial order before display and caches bounded pure results
for the space window lifetime. Configuration, plugin revision and enablement
changes invalidate results. Failed services fall back within a bounded time;
late replies cannot rearrange a replaced owner.

Before packaging use `plugin_project.test` for cold state, saved order across
fresh invocations, explicit separators, absent contextual items and a future
unknown ID. Assert actual IDs, captions and kinds. This is not GUI acceptance:
after normal installation also test first-open layout, save/restart and disable.

Use `ctx.runtime.describe()` to check that `ui.toolbar-layout` is present before
advertising this feature. This is a service protocol, not a new plugin format or
a special case for a particular plugin.

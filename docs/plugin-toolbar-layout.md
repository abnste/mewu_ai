# Toolbar layout service

A portable plugin can contribute a generic layout service using
`ctx.services.register({id, title, metadata: {surface: 'toolbar-layout', targets: ['capture.selection']}}, handler)`.
It requires `ui.render`. The host invokes it with
`{type: 'layout', surface: 'capture.selection', items: [{id, title}, ...]}`.
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

Use `ctx.runtime.describe()` to check that `ui.toolbar-layout` is present before
advertising this feature. This is a service protocol, not a new plugin format or
a special case for a particular plugin.

---
name: mewu-plugin-authoring
description: Create, validate and prepare an independent Mewu JavaScript plugin from a user request, including settings, toolbar layout, Agent, memory and object extensions. Use when the user asks Mewu to build or modify a plugin.
license: MPL-2.0
compatibility: Mewu with skills_list, skill_read and plugin_project tools; no Node.js or shell required. Other agents can follow this workflow with the Mewu SDK and normal package review.
---

# Build a Mewu plugin

Use only the filenames returned by `skill_read.availableFiles`. This skill's
bundled references are `references/sdk.d.ts`, `references/ui-client.d.ts`, and
`references/toolbar-layout.md`. Read them only if needed; `plugin_project.describe`
already returns their full current content. Do not look for additional examples
or guessed paths. Once the contract is available, write the plugin, then correct
specific validation errors. A missing file is not a reason to search personal
history or repeatedly try alternate filenames.

1. Read the current contract using `plugin_project` with `op: "describe"`. It is the installed host's SDK, not a remembered or invented API. Match the requested extension point before writing code. If the host lacks the requested capability, explain that limitation; do not replace a requested native integration with an unrelated HTML mockup.
2. Create one independent project using `plugin_project` with `op: "write"`, a short lowercase `projectId`, and `project: {manifest, modules}`. `manifest` is the complete `mewu-plugin.json`; `modules` maps relative `.mjs` filenames to UTF-8 JavaScript source. Use `expectedSha256: null` for a new project. Read an existing project before editing and use its returned SHA-256. Keep the same plugin ID and project ID when updating it, and increment its semantic version before installing changed code. The tool validates manifest, imports and JavaScript syntax before saving atomically. Correct errors from the tool, never claim a failed write succeeded.
3. Use `@mewu/sdk` and exported `activate(ctx)`. Run in the host's bounded JavaScript runtime; no Node.js, filesystem imports, eval, unapproved shell, arbitrary network or access to other plugins. Request only the actual permissions required. Settings belong to an ordinary `ui` service with `metadata.surface: "settings"`; keep business logic in this plugin. Its handler returns `{html,width,height}` for `input.type === "mount"`; do not call `ctx.ui.render` for a settings page. The host injects the global `mewu` client into the HTML: await `mewu.ready`, then `mewu.call(input)` invokes this same settings handler. Never embed the native SDK in the HTML. Shared settings use `configuration.get` and `configuration.compareAndSet` from that settings surface. Subscribe to `mewu.on('appearance', value => ...)` for host colors. Render using the SDK UI client and the host theme, not a separate application.
4. Run `plugin_project` with `op: "test"`, projectId, expectedSha256 and 1–8 `cases: [{serviceId, input, assertions:[{path,equals}]}]`. Paths are JSON Pointers into the returned value. Optional `configuration` supplies simulated initial values, never real user settings. Every case uses a fresh interpreter; simulated configuration alone persists between cases. Only runtime/catalog reads and simulated configuration are available; unavailable capabilities produce explicit failures, never fake success. Inspect results and fix failed cases yourself, then repeat write/test with the new SHA. Cover cold state, save then read in a fresh invocation, contextual/unknown IDs and separators. Never use assertions that merely match the buggy implementation instead of the user's request. `write` validates syntax/imports only, NOT correctness.
5. Use `plugin_project` with `op: "prepare"`, projectId and the returned expectedSha256 to create an installable ZIP in **Plugins → Installed → Self-made**. A failed service check for the same source blocks preparation. `serviceTests: null` means untested, not passed. Preparation does NOT install or enable it; normal source/permission review remains required.
6. After installation, test the requested actual UI, settings persistence after restart, disabling and removal. Isolated service tests do not prove DOM rendering or native integration. State exactly which checks passed and which remain untested. Never describe a prepared package as a completed/verified feature.

## Toolbar ordering

Use the host's `toolbar-layout` service contract in `describe`, with the existing toolbar's IDs. The service returns only the ordered IDs; it does not create replacement buttons or modify host actions. New or omitted buttons remain visible. Keep the editable order in plugin configuration, expose ordering controls in the plugin's settings and read that same configuration in its layout service. Do not hardcode a plugin ID in the host.

Read `toolbarCatalog` and `ctx.ui.toolbars.describe(surface)` rather than inventing a catalog. Use host captions (including localization), explicit `kind`, and include separator rows in editing and persistence. Module globals do not survive calls: observing `input.items` during layout cannot populate a later settings invocation. Test cold settings before any screenshot, separator moves, missing contextual controls, new unknown controls and saved order restoration. If a required generic API is missing, report that gap instead of replacing it with guessed names or a private host patch.

## Portable skills

Mewu reads normal Agent Skills folders from its data directory's `skills/`. Copy an existing portable skill folder there, keeping SKILL.md and references/scripts/assets together. A skill's `allowed-tools` field is advisory; it cannot grant permissions or create tools. If a skill requires a tool not available in this run, report the missing capability. Reading a script does not execute it. Never search personal history to discover APIs: use the current SDK contract and skill references.

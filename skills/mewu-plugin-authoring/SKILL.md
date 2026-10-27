---
name: mewu-plugin-authoring
description: Create, validate and prepare an independent Mewu JavaScript plugin from a user request, including settings, toolbar layout, Agent, memory and object extensions. Use when the user asks Mewu to build or modify a plugin.
license: MPL-2.0
compatibility: Mewu with skills_list, skill_read and plugin_project tools; no Node.js or shell required. Other agents can follow this workflow with the Mewu SDK and normal package review.
---

# Build a Mewu plugin

1. Read the current contract using `plugin_project` with `op: "describe"`. It is the installed host's SDK, not a remembered or invented API. Match the requested extension point before writing code. If the host lacks the requested capability, explain that limitation; do not replace a requested native integration with an unrelated HTML mockup.
2. Create one independent project using `plugin_project` with `op: "write"`, a short lowercase `projectId`, and `project: {manifest, modules}`. `manifest` is the complete `mewu-plugin.json`; `modules` maps relative `.mjs` filenames to UTF-8 JavaScript source. Use `expectedSha256: null` for a new project. Read an existing project before editing and use its returned SHA-256. Keep the same plugin ID and project ID when updating it, and increment its semantic version before installing changed code. The tool validates manifest, imports and JavaScript syntax before saving atomically. Correct errors from the tool, never claim a failed write succeeded.
3. Use `@mewu/sdk` and exported `activate(ctx)`. Run in the host's bounded JavaScript runtime; no Node.js, filesystem imports, eval, unapproved shell, arbitrary network or access to other plugins. Request only the actual permissions required. Settings belong to an ordinary `ui` service with `metadata.surface: "settings"`; keep business logic in this plugin. Its handler returns `{html,width,height}` for `input.type === "mount"`; do not call `ctx.ui.render` for a settings page. The host injects the global `mewu` client into the HTML: await `mewu.ready`, then `mewu.call(input)` invokes this same settings handler. Never embed the native SDK in the HTML. Shared settings use `configuration.get` and `configuration.compareAndSet` from that settings surface. Subscribe to `mewu.on('appearance', value => ...)` for host colors. Render using the SDK UI client and the host theme, not a separate application.
4. `write` already validates the complete module graph without executing it. Use `plugin_project` with `op: "prepare"`, projectId and the returned expectedSha256 to create the installable ZIP. This prepares a draft in **Plugins → Installed → Self-made**. It is NOT installed or enabled until the user reviews the ordinary install screen. Never bypass the permission/source/version review or report a draft as an installed plugin.
5. After installation, test the requested interaction, settings persistence, disabling and removal. Build-time validation is not an interaction test. Explain precisely what has and has not been tested.

## Toolbar ordering

Use the host's `toolbar-layout` service contract in `describe`, with the existing toolbar's IDs. The service returns only the ordered IDs; it does not create replacement buttons or modify host actions. New or omitted buttons remain visible. Keep the editable order in plugin configuration, expose ordering controls in the plugin's settings and read that same configuration in its layout service. Do not hardcode a plugin ID in the host.

## Portable skills

Mewu reads normal Agent Skills folders from its data directory's `skills/`. Copy an existing portable skill folder there, keeping SKILL.md and references/scripts/assets together. A skill's `allowed-tools` field is advisory; it cannot grant permissions or create tools. If a skill requires a tool not available in this run, report the missing capability. Reading a script does not execute it. Never search personal history to discover APIs: use the current SDK contract and skill references.

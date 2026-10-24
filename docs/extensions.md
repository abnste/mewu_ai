# Executable extensions (API)

Mewu keeps its Rust/Tauri screenshot interface. API extensions contain executable JavaScript modules, run on demand in a separate QuickJS worker, and use a versioned host bridge. Installing a plugin does not install Electron, Node.js or Python. Community plugins use executable JavaScript packages; JSON-only declaration packages are no longer accepted. Existing user data and built-in module choices are preserved.

Built-in modules retain their historical on-disk metadata for user preferences and data audits. That metadata is not an installable plugin API. There is no declaration-package importer, remote declaration catalog or declaration update command; retained third-party declarations are excluded from capability snapshots and rejected at execution. The executable package's `schemaVersion` and `hostApi` fields remain versioned for validation, under the single public name **Mewu API**.

The community catalog is maintained separately at [abnste/mewu-plugins](https://github.com/abnste/mewu-plugins). Authors keep their code and versioned ZIP releases in their own GitHub repository and submit an entry to the catalog. Catalog entries are discovery metadata; installation downloads the actual ZIP and checks its size, SHA-256 and manifest identity.

## Platforms

One JavaScript ZIP normally runs on Windows, macOS and Linux. Omit manifest `platforms` and use catalog `platforms: ["any"]`; the host supplies the same SDK and isolated worker on each platform. No per-OS rewrite or three packages is required for Agent logic, memory, model/channel protocols or HTML/SVG UI.

If the entire plugin really depends on one OS, use manifest `"platforms": ["windows-x86_64"]` (other targets: `darwin-aarch64`, `linux-x86_64`). The catalog may narrow that allowlist but cannot advertise additional platforms. Empty, duplicate, unknown and mixed `any` lists are rejected. Local ZIPs use the same check as GitHub packages; install, enable, rollback, dependency resolution and each run recheck support. Existing incompatible installations remain removable and retain their data.

A portable plugin can instead narrow just one registration with `metadata: { platforms: ["windows-x86_64"] }`, while keeping its other services portable. `metadata: { requiresRegion: true }` requires the native region adapter, which supports Windows, macOS, and GNOME 50 Wayland x64 with Mewu Desktop Witness and system permission. Unsupported desktops keep those controls disabled. Declaring a region permission does not disable unrelated UI/memory services and never grants missing native capability. The invocation's host-provided `context.platform` is `windows`, `macos` or `linux` for optional behavior inside a handler.

This follows the universal-package/default and explicit native-platform pattern described by [VS Code](https://code.visualstudio.com/api/working-with-extensions/publishing-extension#platform-specific-extensions) and [npm OS allowlists](https://docs.npmjs.com/cli/v11/configuring-npm/package-json/#os).

## A first plugin

Use Node.js 22+ for development. In a separate plugin project, install the published development SDK:

```sh
npm init -y
npm pkg set type=module
npm install --save-dev --ignore-scripts --no-audit --no-fund https://github.com/abnste/mewu-plugins/releases/download/v0.1.0/mewu-sdk-0.1.0.tgz
```

Create `mewu-plugin.json` and `index.mjs` in that project. The host supplies `@mewu/sdk` at runtime; do not include `node_modules` in the ZIP.

```json
{
  "schemaVersion": 2,
  "hostApi": 2,
  "id": "yourname.counter",
  "version": "1.0.0",
  "name": "Counter",
  "description": "A persistent counter",
  "license": "MPL-2.0",
  "runtime": "javascript",
  "entry": "index.mjs",
  "permissions": ["storage.read", "storage.write"],
  "tags": ["tools"]
}
```

```js
import { definePlugin } from '@mewu/sdk';

export default definePlugin(ctx => {
  ctx.commands.register({
    id: 'counter.increment', title: 'Increment',
    metadata: { placement: 'space-toolbar', input: {} },
    inputSchema: { type: 'object', additionalProperties: false }
  }, async () => {
    for (;;) {
      const before = await ctx.storage.get('count');
      const after = (before ?? 0) + 1;
      if (await ctx.storage.compareAndSet('count', before, after)) return { content: `Count: ${after}` };
    }
  });
});
```

`@mewu/sdk` is supplied by the worker, not downloaded when the plugin runs. Its [source and TypeScript declarations](../packages/sdk) can also be used as a development package. Modules may import other relative `.js` or `.mjs` files within their own package. Native modules, external module URLs and Node globals are unavailable.

Use the [packaging tools](../tools/extension-market/README.md) to validate and build a ZIP. By default the package includes only the root manifest and entry module; list additional modules explicitly. Publish an immutable versioned GitHub Release asset, generate its catalog entry, and submit that entry to the community repository. Raising the version is required when package contents change. Follow the [complete author workflow](../tools/extension-market/README.md#from-an-independent-project-to-a-market-pr) to build the validator, pack the plugin, publish its own Release and submit the catalog PR.

## Extension points

All registrations use the same lifecycle and executable handler. A handler receives `(input, {context, host, storage})`; `context` is assigned by the host, including the current Agent and the explicitly referenced screen region selected for the current message.

| Registration | Purpose |
| --- | --- |
| `ctx.commands.register` | User actions; `metadata.placement: "selection-toolbar"` adds a selection action, and `"space-toolbar"` adds an action to the lower-left space controls |
| `ctx.tools.register` | A plugin-defined tool automatically offered to the normal conversation Agent with its description and JSON input schema |
| `ctx.agents.register` | An execution strategy selectable in Agent settings |
| `ctx.memory.register` | A memory implementation selectable for an Agent |
| `ctx.models.register`, `ctx.channels.register` | Model and communication services callable through the service bridge |
| `ctx.renderers.register`, `ctx.ui.register` | Interactive content and interface services |
| `ctx.triggers.register` | Hold-type modifier triggers; see [Hold triggers](#hold-triggers-inputtrigger) |
| `ctx.services.register` | An author's own named service contract; the core need not know its business domain |

Services can declare `inputSchema`. Other plugins use `ctx.services.list` and `ctx.services.call("plugin-id/service-id", input)`. Cross-plugin calls require a declared, compatible dependency; permissions are attenuated across the call chain. Use `metadata.contract` to identify a custom contract and its version. Do not treat module tags as permissions or service protocols.

Space actions receive `metadata.input` when it is a JSON object, otherwise `{}`, and the current scene identity. They receive no implicit region grant. A conversation message can carry the currently selected, explicitly referenced capture region; multiple references do not implicitly authorize the first one. Tools with `metadata.requiresRegion: true` are offered only when that message has a supported live screen region. Imported images, recordings and blackboards remain usable as normal conversation attachments.

Enabled tool registrations join the current Agent's existing tool loop. Mewu validates their schemas, freezes the plugin version and package hash, and rechecks the live registry before dispatch. Execution records identify these calls as extensions, separately from MCP. During tool initialization, only `storage.get`, `storage.list` and `services.list` are available; handlers receive their declared host capabilities only after dispatch is recorded. Stop and Escape use the normal conversation lifecycle.

Select an Agent or memory provider from an existing Agent's settings. Normal conversation persistence and cancellation remain owned by Mewu. A memory provider receives `operation: "recall"` with a query, or `operation: "retain"` with the completed conversation text. The memory switch still controls whether these calls happen. Provider data is isolated by plugin and Agent. Storage is paginated; it has no fixed total record-count limit. `compareAndSet` supports concurrent updates without overwriting another invocation.

A bound Agent runs without requiring a host model connection or API key. It receives visible conversation messages and image artifact identifiers, and may use its own services or declared `network.fetch` permission. A host connection is needed only if it calls `ctx.model.invoke`. The run records the actual plugin version and SHA-256. `ctx.events.emit('agent.text', text)` and `ctx.events.emit('agent.reasoning', text)` stream public output; public reasoning is also retained in the conversation. Provider-private protocol state is not exposed through these APIs.

Each invocation receives a bounded recent conversation window, preserving the Agent soul and current request. `context.history` reports total, included and omitted message counts. This limits a single request rather than the durable conversation or memory collection; full history stays in Mewu, and the selected memory provider retains data independently across runs.

`ctx.model.invoke` uses the current conversation's selected model and host-held credentials. It accepts visible messages, a prompt and image artifact identifiers. No connection key is passed to plugin code. A plugin can implement its own observation/action loop instead of inheriting the built-in Agent's loop.

`ctx.ui.render({html, width, height})` creates a persistent HTML object in the current space. It uses the existing isolated, movable object renderer and blackboard behavior. HTML/SVG/Canvas can be interactive inside the sandbox; it does not receive unrestricted Tauri IPC or access to the application's DOM.

## Hold triggers (`input.trigger`)

A plugin activation can declare hold-type triggers: while the user holds a bare modifier key (`Alt`, `Control`, `Shift`) or an exact modifier combination such as Ctrl+Alt, each matching registration receives an ordinary invocation. The space window owns the keyboard listeners; there is no OS-global hook.

```js
export default definePlugin(ctx => {
  ctx.triggers.register({ id: 'yourname.alt-hold', code: 'Alt' }, event => {
    if (event.phase === 'down') { /* begin */ }
    if (event.phase === 'up') { /* event.pointer.x / y, event.modifiers */ }
  });
});
```

The descriptor admits modifiers only: `code` names its primary modifier (`'Alt' | 'Control' | 'Shift'`), and the optional `with` array lists extra modifiers (`'Ctrl' | 'Shift' | 'Alt'`) that must be held together with it. The runtime rejects letters, digits and every other key at registration. Matching is exact set equality, so a bare `Alt` trigger does not fire while Ctrl+Alt is held.

The invocation input carries phase and geometry, never characters: `{type: 'trigger', phase: 'down' | 'up' | 'cancel', reason?: 'escape' | 'blur' | 'modifiers_changed' | 'scene_changed', pointer: {x, y, buttons}, modifiers: {ctrl, shift, alt}}`. `pointer` is the space window's CSS pixels at the moment of the event; `up` reports the modifiers remaining after the release. `Escape` during a hold and a window blur both end it with `cancel`. Adding a modifier cancels the old exact combination and arms the new matching combination regardless of press order. A hold keeps its original scene; switching scenes cancels it instead of committing into the next scene. Disabled or upgraded registrations receive no release event, and newly enabled registrations wait for a fresh keydown. The host owns typing-conflict protection: auto-repeat and composition (`isComposing`) keydowns are ignored, and because bare modifiers do not type, a hold may arm while an input is focused. Trigger delivery requires the manifest permission `input.trigger` and rechecks the live registry like any invocation. With no characters in the payload, a trigger cannot be used as a keylogger.

The independent [hold-trigger example](../extensions/examples/hold-trigger) renders a small canvas badge for every phase so arrivals are visible without instrumentation.

## Region operation example

The independent [region Agent](../extensions/examples/region-agent) registers an operation tool and an optional Agent implementation. Select and reference the region, then send the task through the existing conversation bar. The current Agent chooses whether to call the plugin; there is no separate task button or task-entry dialog. The plugin observes the authorized region, passes the new image artifact to the selected model, interprets the model's decision, performs one action, and obtains another screenshot. The core contains neither that plugin's identity nor its decision algorithm.

The host binds a region grant to a real capture, current selection and application window. Image imports, long screenshots and blackboard images do not authorize desktop input. Each operation rechecks the source, display layout, window identity, input obstruction and grant. Coordinates are relative to the observed region. Keyboard input also requires a verified focus target. Every action requires a successful new observation.

On Windows, layered windows with `WS_EX_TRANSPARENT` pass mouse input to the application underneath and do not block the grant. These overlays may still contribute visible pixels to the authorized region's screenshot; the host does not promise an image containing only the target application's pixels. Observation rechecks the actual hit-tested application, and mouse actions additionally check their hit-tested target immediately before input. These checks do not lock external windows against later changes.

This first input adapter supports **Windows**. macOS and Wayland input grants are not implemented by this adapter and report unsupported rather than borrowing an unrelated screenshot permission. Screenshot and drawing features on those platforms remain independent of executable plugins.

Use the conversation's existing stop button or press Escape during region operations. Disabling, removing or updating the plugin revokes its in-flight authority. An operation already completed in another application cannot be undone by revocation.

The separate [journal memory](../extensions/examples/journal-memory) and [interactive ruler](../extensions/examples/interactive-widget) demonstrate that these interfaces are not specific to desktop automation.

## Host actions (`action.*`)

`ctx.actions.invoke({ action, regionId, variant? })` asks the host to run one of its four existing region actions: `copy` (write the region, always annotated, to the clipboard), `save` (export through the native save dialog), `pin` (create a floating pinned window using the currently enabled pin contribution) and `reference` (add the region to the current conversation's references; an already-referenced region succeeds without changes). `variant: 'annotated' | 'original'` (default `annotated`) applies to `copy` and `save` only.

Each action name maps to its own declared permission, dispatched per call by the host: `copy` → `action.clipboard`, `save` → `action.export`, `pin` → `action.pin`, `reference` → `action.reference`. An undeclared permission is rejected before any action starts (`HOST_PERMISSION_DENIED`).

The region id comes from the host-provided invocation context — typically a `selection-toolbar` command's `context.regionId`. The space window re-checks that the region still exists in the current scene and that the host is not busy (an export in flight, recording, long screenshot or exit preparation), then runs exactly the same action implementation as the first-party toolbar. A failed or user-canceled action surfaces as a host call failure (`HOST_ACTION_FAILED`) with a fixed reason such as `已取消`, `宿主正忙` or `选区不存在或已关闭`; Requests are bound to the originating scene and consumed once by native code. Stopping/disabling the originating plugin or a 30-second preparation deadline revokes pending work, including a save dialog; late replies cannot start another export. Native image publication and pin readiness recheck the same lease. An already-published image or live pin is not undone retroactively. A missing post-publication acknowledgement is reported as an unconfirmed result, never automatically retried. This complements `region.observe`/`region.action`, which automate authorized desktop input: host actions work on every platform, operate on the visible scene and never grant input authority.

The [region-actions](../extensions/examples/region-actions) example registers two selection-toolbar commands that copy and pin the current selection through this facade.

## Lifecycle and permissions

The development host also supports settings services: register a `ui` service with
`metadata: {surface: 'settings'}` to add its own sidebar entry. The existing
isolated document bridge delivers `mount` and subsequent client calls, with no
scene or screenshot authority. Closing or disabling it revokes its generation.
`configuration.get/compareAndSet` share plugin-owned settings across Agent roles;
writes require a live settings surface and the declared configuration permission.

`process.rpc` explicitly admits installed local executables over bounded JSON-lines
stdio. Processes belong to one invocation and close on completion, cancellation
or revocation; no background Node runtime is bundled. Native programs run under
the user's normal account and inherit its environment, so this permission is not
an OS sandbox. Protocols, arguments and UI belong to the plugin. The independent
[External Agents plugin](../extensions/official/agent-connectors/README.md) is an
example. Current-input image bindings additionally require `media.read`; only
host-owned frame handles can fill null protocol fields. Use `ctx.runtime.describe()`
to check the required capabilities before registering services on older hosts.

Workers have no ambient file, network, process or native-module API. Host calls require the plugin's declared permission and live version/state checks. Limits on script memory, execution time, protocol messages and individual values protect the application; they are not a promise that arbitrary native code has an OS security sandbox. Third-party native executables are not accepted as API package modules.

`network.fetch` permits HTTPS and HTTP on literal loopback addresses or `localhost`, including an operator's local Agent or memory service. It does not grant unrestricted HTTP, redirects, shell execution or a host credential export. Package authors must implement their service's protocol in JavaScript or delegate to another declared plugin service.

Activations and invocations run off the UI thread. Unload cancels requests, disposes registrations and waits for the worker to exit. The registry, package bytes, rollback version, provider bindings and plugin-owned storage live in `extensions.db` under the user's configured Mewu data directory. They move with that directory; existing conversations, built-in module choices and retired declaration records are preserved. Retired third-party declarations do not grant capabilities or execute.

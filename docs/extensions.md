# Executable extensions (API 2)

Mewu keeps its Rust/Tauri screenshot interface. API 2 extensions contain executable JavaScript modules, run on demand in a separate QuickJS worker, and use a versioned host bridge. Installing a plugin does not install Electron, Node.js or Python. The earlier JSON-only API 1 packages remain supported.

The community catalog is maintained separately at [abnste/mewu-plugins](https://github.com/abnste/mewu-plugins). Authors keep their code and versioned ZIP releases in their own GitHub repository and submit an entry to the catalog. Catalog entries are discovery metadata; installation downloads the actual ZIP and checks its size, SHA-256 and manifest identity.

## A first plugin

Create `mewu-plugin.json` and `index.mjs` in a separate directory:

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
    inputSchema: { type: 'object', additionalProperties: false }
  }, async () => {
    for (;;) {
      const before = await ctx.storage.get('count');
      const after = (before ?? 0) + 1;
      if (await ctx.storage.compareAndSet('count', before, after)) return after;
    }
  });
});
```

`@mewu/sdk` is supplied by the worker, not downloaded when the plugin runs. Its [source and TypeScript declarations](../packages/sdk) can also be used as a development package. Modules may import other relative `.js` or `.mjs` files within their own package. Native modules, external module URLs and Node globals are unavailable.

Use the [packaging tools](../tools/extension-market/README.md) to validate and build a ZIP. By default the package includes only the root manifest and entry module; list additional modules explicitly. Publish an immutable versioned GitHub Release asset, generate its catalog entry, and submit that entry to the community repository. Raising the version is required when package contents change.

## Extension points

All registrations use the same lifecycle and executable handler. A handler receives `(input, {context, host, storage})`; `context` is assigned by the host, including the current Agent and, when explicitly started from a selection, its region identity.

| Registration | Purpose |
| --- | --- |
| `ctx.commands.register` | User actions; `metadata.placement: "selection-toolbar"` adds a selection action |
| `ctx.tools.register` | A plugin-defined callable tool |
| `ctx.agents.register` | An execution strategy selectable in Agent settings |
| `ctx.memory.register` | A memory implementation selectable for an Agent |
| `ctx.models.register`, `ctx.channels.register` | Model and communication services callable through the service bridge |
| `ctx.renderers.register`, `ctx.ui.register` | Interactive content and interface services |
| `ctx.services.register` | An author's own named service contract; the core need not know its business domain |

Services can declare `inputSchema`. Other plugins use `ctx.services.list` and `ctx.services.call("plugin-id/service-id", input)`. Cross-plugin calls require a declared, compatible dependency; permissions are attenuated across the call chain. Use `metadata.contract` to identify a custom contract and its version. Do not treat module tags as permissions or service protocols.

Select an Agent or memory provider from an existing Agent's settings. Normal conversation persistence and cancellation remain owned by Mewu. A memory provider receives `operation: "recall"` with a query, or `operation: "retain"` with the completed conversation text. The memory switch still controls whether these calls happen. Provider data is isolated by plugin and Agent. Storage is paginated; it has no fixed total record-count limit. `compareAndSet` supports concurrent updates without overwriting another invocation.

A bound Agent runs without requiring a host model connection or API key. It receives visible conversation messages and image artifact identifiers, and may use its own services or declared `network.fetch` permission. A host connection is needed only if it calls `ctx.model.invoke`. The run records the actual plugin version and SHA-256. `ctx.events.emit('agent.text', text)` and `ctx.events.emit('agent.reasoning', text)` stream public output; public reasoning is also retained in the conversation. Provider-private protocol state is not exposed through these APIs.

Each invocation receives a bounded recent conversation window, preserving the Agent soul and current request. `context.history` reports total, included and omitted message counts. This limits a single request rather than the durable conversation or memory collection; full history stays in Mewu, and the selected memory provider retains data independently across runs.

`ctx.model.invoke` uses the current conversation's selected model and host-held credentials. It accepts visible messages, a prompt and image artifact identifiers. No connection key is passed to plugin code. A plugin can implement its own observation/action loop instead of inheriting the built-in Agent's loop.

`ctx.ui.render({html, width, height})` creates a persistent HTML object in the current space. It uses the existing isolated, movable object renderer and blackboard behavior. HTML/SVG/Canvas can be interactive inside the sandbox; it does not receive unrestricted Tauri IPC or access to the application's DOM.

## Region operation example

The independent [region Agent](../extensions/examples/region-agent) registers its own selection command and Agent implementation. It observes the authorized region, passes the new image artifact to the model, interprets the model's decision, performs one action, and obtains another screenshot. The core contains neither that plugin's identity nor its decision algorithm.

The host binds a region grant to a real capture, current selection and application window. Image imports, long screenshots and blackboard images do not authorize desktop input. Each operation rechecks the source, display layout, window identity, occlusion and grant. Coordinates are relative to the observed region. Keyboard input also requires a verified focus target. Every action requires a successful new observation.

This first input adapter supports **Windows**. macOS and Wayland input grants are not implemented by this adapter and report unsupported rather than borrowing an unrelated screenshot permission. Screenshot and drawing features on those platforms remain independent of executable plugins.

Stop the plugin from its selection button or press Escape during region operations. Disabling, removing or updating the plugin revokes its in-flight authority. An operation already completed in another application cannot be undone by revocation.

The separate [journal memory](../extensions/examples/journal-memory) and [interactive ruler](../extensions/examples/interactive-widget) demonstrate that these interfaces are not specific to desktop automation.

## Lifecycle and permissions

Workers have no ambient file, network, process or native-module API. Host calls require the plugin's declared permission and live version/state checks. Limits on script memory, execution time, protocol messages and individual values protect the application; they are not a promise that arbitrary native code has an OS security sandbox. Third-party native executables are not accepted as API 2 package modules.

`network.fetch` permits HTTPS and HTTP on literal loopback addresses or `localhost`, including an operator's local Agent or memory service. It does not grant unrestricted HTTP, redirects, shell execution or a host credential export. Package authors must implement their service's protocol in JavaScript or delegate to another declared plugin service.

Activations and invocations run off the UI thread. Unload cancels requests, disposes registrations and waits for the worker to exit. The registry, package bytes, rollback version, provider bindings and plugin-owned storage live in `extensions.db` under the user's configured Mewu data directory. They move with that directory; the original conversation and API 1 databases are preserved.

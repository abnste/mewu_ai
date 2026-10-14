# Mewu extension runtime

`mewu-extension` runs executable API 2 JavaScript plugins with QuickJS 0.14.0.
It does not start Node, Electron, a WebView, or a system shell. The desktop
starts its own executable with `--extension-worker` before application state is
opened. `mewu-extension-worker` is the equivalent standalone development host.

Each worker owns one plugin scope and accepts serial invocations. Pure
ECMAScript, package-relative ES modules, and the embedded `@mewu/sdk` module are
available. Filesystem, native modules, process execution, and network globals
are absent. The desktop authorizes every host call independently; manifest
permissions are requests, not authority.

## Package

```json
{
  "schemaVersion": 2,
  "hostApi": 2,
  "runtime": "javascript",
  "id": "example.region-agent",
  "name": "Region Agent",
  "description": "",
  "version": "1.0.0",
  "license": "MPL-2.0",
  "entry": "index.js",
  "permissions": ["region.observe", "region.input", "model.invoke"],
  "tags": ["agi", "vision"]
}
```

The package root contains `mewu-plugin.json` and UTF-8 `.js`/`.mjs` modules.
Module names are portable relative paths; links, reparse points, drive paths,
case collisions, traversal, bytecode, BOM, and NUL are rejected. Up to 128
modules, 1 MiB each and 8 MiB combined are accepted. The directory loader must
receive an application-owned immutable staging directory. It is not a
filesystem sandbox for a concurrently modified user directory.

## SDK and services

```js
import { definePlugin } from '@mewu/sdk';

export default definePlugin(ctx => {
  ctx.commands.register({ id: 'inspect', title: 'Inspect' }, async (input, run) => {
    const frame = await ctx.region.observe(run.context.regionId);
    return ctx.model.invoke({ prompt: input.prompt, observation: frame });
  });
});
```

`commands`, `tools`, `agents`, `memory`, `models`, `channels`, `renderers`, `ui`,
and `services` all provide `register(descriptor, handler)`. `services` is the
generic extension point for an author's own versioned protocol; it does not
require a new host enum for billing, workflows, or future domains. The handler is plugin code,
not a named built-in engine. Registration is open during activation and then
frozen. Each registration returns a disposer. Provider-specific operations
can use an input operation discriminator and retain state inside the plugin
scope; there is no fixed memory adapter or agent algorithm.

Descriptors contain `id`, `title`, optional `description`, `inputSchema`, and
`metadata`. The runtime supplies `kind`. Schemas and metadata are JSON objects.
Handlers receive `(input, { context, host, storage })`. `context` is the host's
invocation snapshot; a region ID is not itself input authority. Host methods
must verify the live region grant, owner, plugin revision, cancellation, and
current screen topology before each action.

The embedded SDK provides:

- `ctx.host.call(method, params)` for versioned host capabilities.
- `ctx.storage.get/set/delete/compareAndSet/list` for scoped durable data.
- `ctx.model.invoke`, `ctx.region.observe/action` for multimodal loops.
- `ctx.services.list/call` for provider composition.
- `ctx.ui.render`, `ctx.events.emit`, and `ctx.onDispose`.

Image observations and generated artifacts stay structured JSON, for example
`{ type: 'image', artifactId, width, height }`. The desktop resolves only
host-owned artifact handles and performs model-protocol image conversion.
Plugins do not receive arbitrary paths or model credentials.

`storage.list({ prefix?, cursor? })` returns `{ keys, nextCursor }` in pages of
100 and has no fixed total record limit. `compareAndSet(key, expected, value)`
returns a boolean; `expected: null` means the key must not exist, and does not
match a stored JSON null. Non-null expected values use canonical JSON equality.
Inter-plugin `services.call('pluginId/serviceId', input)` requires a declared
SemVer dependency. Its host permissions are the intersection of the caller and
callee grants, with every ancestor's revision rechecked during execution.

## Worker protocol

UTF-8 newline-delimited JSON on stdin/stdout; stdout contains no logs.
Every request/host-call ID is a nonempty identifier of up to 128 bytes.

| Host → worker | Worker → host |
| --- | --- |
| `initialize { id, manifest, modules }` | `initialized { id, registrations }` |
| `invoke { id, serviceId, input, context }` | `result { id, result }` |
| `host.result { id, result }` or `{ id, error: { code, message } }` | `host.call { id, method, params }` |
| `cancel { id? }` | active request `error` and worker termination |
| `shutdown { id }` | `stopped { id }` and termination |

Every envelope additionally has `type` equal to the name in the table.
Activation may await host calls before returning registrations. While a
handler awaits `host.result`, the worker remains alive and pumps Promise jobs.
An input thread delivers cancellation while JavaScript executes a CPU loop.

Default limits are 32 MiB interpreter heap, 512 KiB interpreter stack, 90 s per
activation/invocation, 512 host calls per request, and 64 unresolved host calls.
The host should independently limit I/O durations and kill a nonresponsive
worker. Protocol lines are at most 12 MiB; invocation/result/host parameter
values are at most 1 MiB and 48 nested levels. Any invocation error emits a
sanitized structured error and ends that worker scope. Restart the worker
before retrying. Errors never include plugin source, thrown messages, or stack
traces. Shutdown revokes the JS scope first, runs bounded disposal callbacks,
then discards the interpreter; disposal cannot issue fresh host calls.

This interpreter removes ordinary OS access and contains plugin failures. It
does not claim protection from an unknown QuickJS memory-safety vulnerability.
The desktop bridge and OS isolation remain separate security boundaries.

## Verification

`cargo test -p mewu-extension` executes real JavaScript and real worker process
stdio, including asynchronous host calls, all service kinds, multimodal JSON,
import confinement, resource limits, cancellation, sanitized errors, and scope
disposal. Desktop integrations and native region input require their own
platform tests.

Official runtime API: <https://docs.rs/rquickjs/0.14.0/rquickjs/>.

# Agent runtime

Mewu's native Agent runs in Rust. It does not embed a terminal application or add
Node/Electron to the desktop process. This implementation takes its context
boundaries from [Pi v1.4.0](https://github.com/earendil-works/pi/releases/tag/v1.4.0),
particularly its [compaction](https://github.com/earendil-works/pi/blob/v1.4.0/packages/coding-agent/docs/compaction.md)
and [agent loop](https://github.com/earendil-works/pi/blob/v1.4.0/packages/agent/src/agent-loop.ts).
It is an independent implementation, not a bundled Pi engine.

The native engine now exposes general local execution and file tools, with
durable public tool records in conversation context. This remains an independent
implementation, not Pi capability parity; the [baseline audit](agent-capability-audit.md)
distinguishes the original gaps from subsequent implementation and acceptance.

## Ownership

| Layer | Responsibility |
| --- | --- |
| `mewu-agent-runtime` | Provider-independent budgets, transcript planning, checkpoint fingerprints and bounded tool-result projection |
| `mewu-core` | Complete conversations, run receipts, memory provenance, cancellation fences and persisted context checkpoints |
| `agent_context` | Tool-free summarization using the selected connection; session checkpoint validation and reuse |
| `agent_turn_context` | Compaction between complete tool batches, without replaying side effects or splitting protocol continuations |
| `agent_loop` | Model/tool scheduling, streaming, budgets and durable execution boundaries |
| `provider_transport` | Chat Completions, Responses and Anthropic wire protocols |
| Agent/memory plugins | Replaceable execution strategy or long-term memory provider through existing registrations |

External Agent plugins retain ownership of their own client sessions and context
policies. Native summaries are not imposed on those sessions. `ctx.agents.register`
and `ctx.memory.register` remain the extension boundaries; see [extensions](extensions.md).
Mewu still owns the originating scene, cancellation and plugin revocation. A new
engine must not bypass these boundaries by starting an untracked background run.

## Context and sessions

Agent-initiated local memory writes retain the verified `sourceQuote` itself.
They do not accept a second free-form fact beside that quote. Both creation and
revision updates are checked against the originating user's message; tool output,
attachments and assistant claims cannot be relabelled as user facts. Existing
memory records remain unchanged during an upgrade. Session tool transcripts are
durable independently of long-term memory and must be used for task continuity.

The scene transcript remains complete. A model request is a projection, not the
storage format. Old messages are summarized only when the projected input exceeds
its budget, or the selected historical media exceeds request byte limits. The
current user message and its attachments are never summarized away to make a
request fit. Cuts occur before a user turn, preserving recent exchanges together.

Each connection can set `advanced.contextWindowTokens`. The native fallback is
65,536; this is a local policy default, **not a discovered model capability**.
Reserve is one quarter of that window, capped at 16,384. Recent-history target is
one third, capped at 20,000, further reduced by fixed context and summary space.
This is a history-tail target, not a second cap on the current user message: a
larger current message is kept when it still fits the available input window.
Estimates use ASCII bytes/3, two tokens per non-ASCII BMP character and three per
supplementary character. Images have a fixed 4,096-token allowance. These are
heuristics, not a billing tokenizer or a guarantee about a provider's image cost.
Configure the real model window when it differs. The host-only setting is removed
before sending provider JSON.

Summary requests:

- Use the selected connection and never expose tool definitions, execute tools or
  emit their text as an assistant reply.
- Preserve goals, constraints, decisions, file/record identifiers, confirmed work,
  failures and pending actions. Attachment descriptors are not visual evidence.
- Read bounded UTF-8-safe chunks; reject empty, oversized or tool-calling results.
- Check cancellation before and after each request. They do not automatically
  retry an ambiguous network failure.
- Commit a checkpoint only after successful preparation. Failure leaves original
  history intact. Summaries are quoted historical data, never system instructions
  or persistent memories.

The private SQLite cache has at most one checkpoint per scene. A checkpoint binds
the Agent profile, connection identity/revision, conversation start and exact
message prefix hash. Changing the role, model/policy or source messages prevents
reuse. Restart can reuse a matching checkpoint without another summary request.
Deleting/resetting a conversation removes its checkpoint. The storage migration
adds a table without rewriting the old scene payload. Storage generation 13 is
fenced against older writers; keep a cold backup when testing a downgrade.

During tool execution, compaction happens only after the entire batch has settled.
It replaces the completed batch's wire continuation with quoted observed evidence.
It never manufactures pending tool calls, persists private reasoning signatures,
or retransmits a tool merely to rebuild context. Original execution receipts stay
in the journal. Initial image inputs are not duplicated into a permanent second
runtime transcript.

The summary ceiling is not reserved unconditionally. Between tool batches the
host measures the retained request prefix, including images and the continuation
envelope, then bounds the summary to the remaining input space. A smaller summary
may therefore continue an image-heavy task. A genuinely oversized immutable input
still fails explicitly; the host does not drop current frames, raise the model
window silently, or truncate a summary into a false account of executed work.

`session_read` provides paginated access to original text from the **current
conversation**, including messages omitted from the projection. It cannot read
another scene, private reasoning, credentials or file contents. This remains
available when long-term memory is off. Skills and plugin authoring continue to
use their existing bounded tools and permission checks.

## Memory and failures

`read`, `write`, `edit`, `ls`, `find` and `grep` operate on actual local files.
`bash` is discovered from the installed shell; Windows additionally discovers
PowerShell, including the system fallback. The engine does not bundle Node or
Git Bash. A scene has a default workspace under the selected data root; explicit
paths and command `cwd` use the normal OS account. A working directory is **not**
a filesystem sandbox. Skills return their base directory so their scripts can
use these tools. Querying the OS uses an ordinary command, not `system_info` or a
generated plugin. Plugin authoring is reserved for requested plugin work.

Shell invocation owns its Job/process group, stdout and stderr. Nonzero exit,
argument errors and timeouts become model-visible results. Revocation is checked
while waiting to start and while running; cancellation stops the process tree.
The model receives a bounded UTF-8 tail; the output file retains up to 32 MiB.
Display previews are batched every 40 ms, with bounded UTF-8 tails and retained
stdout/stderr ordering. This does not trim the complete output file or invent
a successful result; it prevents tiny pipe reads from flooding the WebView.
Detached background services are not yet a managed Agent capability: a child
holding pipes after its shell exits is stopped and reported, never called a
success. A separate two-command pool prevents persistent MCP connections from
starving shell execution. Both pools participate in shutdown and storage drains.

Schema 13 binds original tool arguments to their receipt hash and stores public
model-visible results, including failures. Subsequent context includes recent
observations in its token accounting and compaction. `session_read` can page the
complete record with `runId`, `toolEntryId` and `offset`. This is historical
evidence, not a replayable provider continuation. Missing legacy records remain
explicit; private reasoning is not retained in this table.

Native, visual, extension and MCP tools share the same model catalog limits.
Native definitions are included before preflight validates the catalog, rather
than appended after a conversation has already begun.

Long-term memory, context summaries and conversation history are separate stores.
Compression never creates or deletes a memory. Existing memory providers and
source-quote/revision rules remain authoritative. An unavailable optional memory
plugin is reported to the model using a fixed host diagnostic, without forwarding
raw remote errors; the current task can continue, but the model must not claim
that the memory store is empty or that a write succeeded.

Correctable tool errors are returned to the model. Unknown execution outcomes,
lost authorization and cancellation stop the run; they are not automatic retries.
An observed result too large for context becomes an explicit omission envelope
with its original byte size and SHA-256. That envelope makes no claim that the
operation succeeded, or that the full output can be recovered. A narrower read is
appropriate where supported; repeating a mutation is not.

The model loop and durable dispatcher share the same 12-round/32-call limits.
The last model round cannot dispatch tools whose results it cannot consume.
This removes the former mismatch where the loop allowed 32 calls but the storage
layer rejected call 17. Budgets remain bounded rather than allowing unbounded
retries or recursive summarization.

## Verification

Regression coverage includes checkpoint restart and canceled writes, migration
payload preservation, exact-prefix invalidation, Chinese/emoji chunking, current
input retention, historical media budgets, session isolation, all three provider
protocols, tool-free compaction, non-reexecution, bounded large results, and the
32/33-call dispatcher boundary. Synthetic HTTP tests exercise the real protocol
adapters; they are not evidence of live-provider or three-platform GUI acceptance.
An opt-in native smoke test can also read one explicitly selected connection
without migrating the live database, summarize a synthetic task and verify an
exact task identifier in a resumed answer. It sends no real conversation,
attachment or tool action and is excluded from normal offline test runs.

General execution lives in the `mewu-agent-tools` crate and the native owned
process adapter. It does not grant extensions additional permissions or turn a
skill document into executable authority. Engine replacement remains a plugin
contract; native tool implementation is independent of plugin-authoring logic.

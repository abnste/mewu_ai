# Agent evaluation

Use the actual native Agent, selected connection, skills and tool loop. A model's
claim of success is not a passing grade. Keep the first failed run and its tool
diagnostics; repair the host contract or workflow and rerun with a new test run.
Do not replace model-produced arguments or files with handwritten successes.

## Layers

1. Offline contracts: schema validation, tool error feedback, cancellation,
   memory isolation, checkpoint reload, media budgets, complete tool batches and
   provider protocol adaptation. Run the runtime/core and native desktop suites.
2. Live Agent tasks: use synthetic pictures, short videos and isolated conversations
   through the installed application's public CLI or original conversation bar.
   Verify actual scene objects, execution receipts and output files. Store source
   commit, platform, model, skill hashes, run IDs, failures and assertions privately.
3. Native GUI: inspect the rendered image/video, undo/redo, reopen and exported
   result on Windows, macOS and Ubuntu. CLI success is not GUI evidence.

## Required task matrix

| Task | Assertions |
| --- | --- |
| Read workflow | Relevant skill is discoverable and loaded; unsupported tools are not invented |
| Image boxes/arrows/text | Correct current target, objects persisted, visible readable pixels, batch undo and redo |
| Worksheet/formula/table | Literal text and rich content use their respective protocols; complete readable layout |
| Video annotation | Current video and sent frame handles, ordered interval, visible only within that interval |
| Text-only request with media | Answer without creating annotation objects |
| Malformed tool arguments | No partial mutation; concrete error reaches model; corrected call can complete |
| Cancel or revoke | No new mutation/model request after authority is lost; uncertain effects are not replayed |
| Conversation compaction | Goal survives, original history readable, no dropped current input or private reasoning leak |
| Memory off/unavailable | Current session still usable; unavailable memory is not reported as empty or saved |
| Plugin authoring | Real model-generated project, service checks, ordinary install review; no fabricated GUI success |

Use normal skills and tools rather than forcing a model-specific backend. Keep
credentials and personal histories out of fixtures, reports and published assets.
Run counters, repeated trials and concrete checks should be reported separately
from subjective visual review. Never label skipped platform checks as passed.

`scripts/agent-eval.mjs` runs an explicit JSON case manifest against an installed
Mewu CLI. Pass `--cli`, `--cases`, a new `--output` directory, the actual `--agent`
and `--connection` IDs, and `--allow-model-requests`. Cases specify `name`, `prompt`,
optional fixture `files`, and `expectKinds`, `expectTools`, `expectText` or
`expectNoObjects`. It creates isolated new conversations, records real run/scene
receipts, and checks saved objects rather than trusting the final answer. A failed
run keeps its CLI error details. The suite opens each test scene because video
sampling requires the visible space's source lease. It rejects external-engine
bindings and mismatched model connections instead of grading them as native runs. A failed
run stops the suite and remains available; observing a timeout does not resend an
operation. Evidence is private and must not be bundled into the release. GUI and
pixel/time-range checks remain separate mandatory work.

# Agent skills and plugin projects

The built-in Agent reads portable Agent Skills folders from `skills/` inside
Mewu's selected data directory (normally `~/.mewu`). A skill has a `SKILL.md`
with YAML `name` and `description`, plus optional `references/`, `scripts/` and
`assets/`. Copy the whole folder when importing a skill from another agent.

Only skill metadata enters the initial model context. `skills_list` discovers
skills; `skill_read` loads a chosen skill or an actual relative reference file.
Its response lists available filenames, so the model need not guess paths.
The format follows [Agent Skills](https://agentskills.io/specification).
Private tools from another agent do not become available just by importing its
skill, and reading a script does not execute it. Skill metadata grants no
permissions. Links and paths outside the selected skill are rejected.

The bundled `mewu-plugin-authoring` skill guides the model through the current
SDK, independent source generation, validation and packaging. Factory files
update only while their content still matches the recorded factory hash;
user modifications remain intact. SDK references are generated from the same
source as the host. `plugin_project.describe` remains the authority for the
currently running host's contract.

The native Agent also bundles `mewu-image-annotation` (image/worksheet, drawing,
formula and table workflows), `mewu-video-annotation` (sampled-frame evidence and
timed annotations), and `mewu-task-verification` (observable completion, error
correction and continuation). These are ordinary portable skill folders, installed
through the same factory-hash mechanism. They load on demand, not as a full prompt
on every turn. Current tool schemas still own target handles, coordinate units,
atomic validation and permissions; a skill cannot add a missing annotation grant.
Editable visual tools remain native so manual drawing and existing exports work
even without a model or without reading a skill.

`plugin_project` supports describe, list, read, write, test and prepare. Writes use
the prior project SHA-256 and an atomic replacement, and validate the manifest
and complete JavaScript module graph without executing it. Errors return to
the model. Source projects live in `plugin-projects/` in the same data directory.
Transport supports bounded tool arguments up to 512 KiB; each operation applies
its own smaller payload/schema limits. An incomplete model stream never runs.
Authoring arguments derive their JSON Schema from the same types used by the
validator; invalid input returns its field path and expected type to the model.
`test` runs declared service cases with JSON-pointer assertions in the real
JavaScript runtime. Each case gets a fresh interpreter; only simulated plugin
configuration persists between cases. It cannot access live data, credentials,
network, processes or native actions. Failed checks return to the Agent for
correction and block preparing those exact source bytes. Passing these checks
does not prove GUI or platform compatibility; installation and actual interaction
still need verification. Changing the source invalidates its previous results.
Service inputs and assertion expectations use explicit JSON text fields
`inputJson` and `equalsJson`. This preserves arbitrary nested arrays and value
types across model providers without guessing or coercing malformed values.
For example, `equalsJson: "true"` asserts a boolean; `equalsJson: "\"true\""`
asserts a string. Reports show the received input and actual/expected types,
including the distinction between a missing path and JSON null.
The native workflow permits 12 model rounds and 32 executed tool calls within
the existing time and output budgets. The execution loop and durable journal
share the round limit; work already saved remains available if a run stops.

Preparing a project produces a normal plugin ZIP in **插件 → 已安装 → 自制**.
It does not mean the package has been installed or its interaction tested.
Installation uses the existing source/version/permission review. Disabling,
removing and updating a generated plugin follow the same lifecycle as other
independent plugins.

For ordering existing screenshot controls, use the generic
[toolbar layout service](plugin-toolbar-layout.md). The plugin owns its UI and
saved preferences; the host moves its existing controls through a keyed list.
It does not create a special toolbar or add a branch for any plugin ID.

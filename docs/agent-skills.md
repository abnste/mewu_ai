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

`plugin_project` supports describe, list, read, write and prepare. Writes use
the prior project SHA-256 and an atomic replacement, and validate the manifest
and complete JavaScript module graph without executing it. Errors return to
the model. Source projects live in `plugin-projects/` in the same data directory.
Transport supports bounded tool arguments up to 512 KiB; each operation applies
its own smaller payload/schema limits. An incomplete model stream never runs.

Preparing a project produces a normal plugin ZIP in **插件 → 已安装 → 自制**.
It does not mean the package has been installed or its interaction tested.
Installation uses the existing source/version/permission review. Disabling,
removing and updating a generated plugin follow the same lifecycle as other
independent plugins.

For ordering existing screenshot controls, use the generic
[toolbar layout service](plugin-toolbar-layout.md). The plugin owns its UI and
saved preferences; the host moves its existing controls through a keyed list.
It does not create a special toolbar or add a branch for any plugin ID.

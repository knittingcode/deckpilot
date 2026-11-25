# Variables: the unified registry

Status: **implemented** in three stacked changes: the registry and module
declarations, the single resolver with protocol 9, and the Variables page
and picker. This document records the audit of the variable code
on `main` at `592d5f1` and the target design of the unified variable
registry. It extends `docs/action-architecture.md` (§4–§11), which stays the
reference for shared variables, the reference grammar and the resolution
pipeline of actions.

## 1. Audit (main at 592d5f1)

### 1.1 How variables are declared and who owns them

| Kind | Declared in | Owner today | Resolved by |
|---|---|---|---|
| System variables `system.…`, `workflow.…` | the closed enum `SystemVariable` in `crates/deckpilot-core/src/variables/expression.rs` (paths, types, descriptions hard-coded) | the core, for all of them, including project and Work Session values | `deckpilot_core::resolution::system_value` from the `ResolutionContext` and the profile |
| Work session `system.work.session.…` | `WorkSessionVariable` in `crates/deckpilot-core/src/work_session.rs`, wrapped by `SystemVariable` | the core enum; values only the agent knows | the agent: `apps/agent/src/service/variables.rs::work_session_value` from the live `WorkSessionSnapshot` (through `VariableStore`, the "shared" resolver) |
| Shared variables `shared.<key>` | the configuration database (schema 8: `shared_variables`, `shared_variable_values`), `crates/deckpilot-storage/src/variables.rs` | the user (`module_id IS NULL`); the store can also hold module-declared definitions (`declare_module_variables`), which no module uses | `SharedVariables::resolve` on a per-profile snapshot (`select_value`: execution > project > profile > global > default) |
| Node outputs `steps.<node>.output.<key>` | the workflow graph | the workflow | the executor's outputs |
| Workflow variables `steps.vars.output.<key>` | the workflow (`WorkflowVariable`) | the workflow | the executor |

There is no module declaration of variables: `ModuleManifest` has no
variables, and nothing ties `system.project.current.…` to the projects
module or `system.work.session.…` to the Work Session module. Disabling the
projects module does not affect `system.project.current.…`; disabling a
module that declared shared variables makes them `permissionDenied`
(`VariableStore::set_disabled_modules`).

### 1.2 Every identifier exposed today, and its canonical id

| Current id | Type | Canonical id (owner) | Status |
|---|---|---|---|
| `system.project.current.id` | projectId | `projects.current.id` (projects) | alias, deprecated since 0.6.0 |
| `system.project.current.key` | string | `projects.current.key` (projects) | alias, deprecated since 0.6.0 |
| `system.project.current.name` | string | `projects.current.name` (projects) | alias, deprecated since 0.6.0 |
| `system.profile.current.id` | profileId | `system.profile.current.id` (core) | unchanged |
| `system.profile.current.name` | string | `system.profile.current.name` (core) | unchanged |
| `system.device.current.id` | string | `system.device.current.id` (core) | unchanged |
| `system.platform` | string | `system.platform` (core) | unchanged |
| `workflow.id` | string | `workflow.id` (core) | unchanged |
| `workflow.execution.id` | string | `workflow.execution.id` (core) | unchanged |
| `system.work.session.id` | string | `workSession.id` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.state` | string | `workSession.state` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.startedAt` | string | `workSession.startedAt` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.elapsedSeconds` | integer | `workSession.elapsedSeconds` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.projectId` | projectId | `workSession.projectId` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.canFinish` | boolean | `workSession.canFinish` (work-session) | alias, deprecated since 0.6.0 |
| `shared.<namespace>.<name>[.…]` | as defined | unchanged (user) | unchanged |
| `steps.<node>.output.<key>` | text | unchanged (workflow) | unchanged |
| `steps.vars.output.<key>` | as declared | unchanged (workflow) | unchanged |

Key labels also keep their own placeholders `{profile}`, `{project.key}` and
`{project.name}` (`face::render_label`); they are not references and are not
changed.

### 1.3 Resolution pipeline today

1. A key press (`device_runtime.rs::dispatch`), a workflow node
   (`graph/executor.rs`), a trigger or an inline invocation reaches
   `Engine::run_action_with` with an `Invocation` (device, workflow, run).
2. `Invocation::context` builds the `ResolutionContext` (profile, project and
   its source, device, workflow, execution, phase).
3. `Engine::resolver` builds a `ContextResolver { profile, os, shared,
   outputs }`:
   - `system.` and `workflow.` → `system_value` from the context;
   - `system.work.session.…` and `shared.` → the injected `shared` resolver
     (the agent's `VariableStore`), confined to the profile;
   - `steps.` → the run's outputs.
4. `resolve_parameters` binds whole-value references (`bind`, native types),
   renders templates as text, collects sensitive values in a `Redactor`,
   then `ActionType::normalize` validates.
5. Previews (`previewReferences`) use the same `ContextResolver` with the
   database's current shared variables and `preview` (redaction, type
   check). The catalog (`listVariables`) is built by the agent's
   `catalog()` from `SystemVariable::ALL` and the stored definitions.
6. Graph validation (`graph/validate.rs`) and `ActionType::normalize` check
   static types of `system.` references (`SystemVariable::value_type`).

### 1.4 Caching

- **Agent, runs:** `VariableStore` keeps one `SharedVariables` snapshot per
  profile, taken at the last reload (`reload_variables` after
  `notify_reload`); a run started after a reload reads the new values.
  `reload` returns the changed keys, announced as `sharedVariablesChanged`.
- **Agent, previews:** read the database directly on every request.
- **Work session:** never cached; the snapshot is read from the watch
  channel at each resolution and elapsed time is computed then.
- **System variables:** never cached; computed from the context.
- **App:** `useCatalog`/`usePreviews` (`components/variable-picker/
  use-variables.ts`) hold the last answer and refresh on the agent's context
  events (`agent-variables` Tauri event), with a generation counter that
  drops obsolete answers. No polling.

### 1.5 IPC and events

Protocol 7 added `listVariables` (metadata only), `previewReferences`, and
the events `projectChanged`, `profileChanged`, `sharedVariablesChanged`
(keys only; also the `system.work.session.…` paths that changed),
`resourceBindingsChanged`, `deviceContextChanged`,
`executionContextChanged`. Protocol 8 added the Work Session requests and
`workSessionChanged`. Responses are adapted to older clients by
`Response::for_protocol`; events newer than the client's protocol are not
sent (`server.rs`).

### 1.6 Frontend

- Variable Picker (`components/variable-picker/`): groups Shared / System /
  Profile / Project / Workflow / Step outputs, search, type filter, live
  previews from the agent; falls back to the static `SYSTEM_ENTRIES` in
  `lib/variables.ts` for agents before protocol 7.
- Shared Variables settings page (`pages/shared-variables.tsx`): a list of
  user variables with an editor, plus a separate read-only "System
  variables" card from the static `SYSTEM_VARIABLES` in
  `lib/shared-variables.ts`.
- `workflow-builder/variable-reference.ts` holds another static table of
  system variable types.

## 2. Target design

### 2.1 Owners and namespaces

- **Core** owns only genuine system variables, in the namespaces `system`
  and `workflow`: `system.profile.current.id`, `system.profile.current.name`,
  `system.device.current.id`, `system.platform`, `workflow.id`,
  `workflow.execution.id`.
- **Modules** own the variables they declare, inside their namespace: the
  module id in lower camel case (`projects` → `projects`, `work-session` →
  `workSession`, `module_namespace`). A reference to a module variable has
  the module namespace as its root: `{{ projects.current.key }}`. Only the
  namespaces of modules that declare variables (`MODULE_NAMESPACES`:
  `projects`, `workSession`) are roots of the grammar; any other root is
  rejected when parsed and validated, exactly as before (`{{ env.home }}`
  stays invalid). A module that declares variables adds its namespace
  there; a test keeps the list equal to the built-in modules', plus
  `projects`, which stays a root for the Projects plugin.
  - Projects owns `projects.current.id`, `projects.current.key`,
    `projects.current.name`. Since WP-C6 the built-in `projects` module is
    removed: the agent publishes them for the Projects plugin
    (`deckpilot.projects`) while it is enabled, under the same ids
    ([module-migration.md](marketplace/module-migration.md) X7).
  - Work Session owns `workSession.id`, `.state`, `.startedAt`,
    `.elapsedSeconds`, `.projectId`, `.canFinish`.
- **Users** own shared variables, `shared.<key>`. The user namespace stays
  open except the reserved prefixes `steps`, `system` and `workflow` (D4,
  "open except reserved"): a user variable `shared.projects.favorite` is
  fine; it can never collide with a module id because shared variables
  always have the `shared.` root.
- **Plugins** (experimental, `docs/marketplace/agent-integration.md`) own
  the variables they declare under the root `plugins`:
  `plugins.<publisherCamel>.<nameCamel>.<path>` (`acme.github-prs` →
  `{{ plugins.acme.githubPrs.open }}`, 1 to 3 path segments). This root is
  not a list: such a reference parses whenever it names a valid plugin id,
  on every computer, and resolves as unavailable (owner: the plugin id)
  where the plugin is not installed or cannot run. Values are what the
  plugin published; no plugin code runs during a resolution.
- Reserved roots that no module may use as its namespace: `steps`,
  `shared`, `system`, `workflow`, `plugins`. A module named after one (the
  `system` module) cannot declare variables. Shared variable keys are not
  affected (`shared.plugins.x` stays a shared variable).

### 2.2 Declarations and the registry

`crates/deckpilot-core/src/variables/registry.rs`:

- `VariableDeclaration { id, name, description, valueType, scope,
  requires, sensitive, readOnly }`. `scope` (`DeclaredScope`) says what the
  value depends on (machine, profile, project, device, workflow, execution,
  workSession); `requires` lists what must be in the context
  (`Requirement`). Core and module variables are always read-only.
- `ModuleManifest::variables` holds a module's declarations;
  `FeatureModule::variable_source` returns the `VariableSource` that
  resolves them.
- `VariableRegistry` keeps the core's variables and every registered
  module's, by canonical id, with their owner. `ModuleRegistry` keeps it in
  step with the module lifecycle: `register` validates the declarations
  (namespace, reserved namespace, one owner per namespace, no duplicate id,
  read-only, a source exists) and refuses the whole module otherwise;
  `unregister` removes the module and its variables; disabling a module (or
  running on an unsupported platform) marks its variables unavailable.
- A canonical reference whose module is disabled, unsupported or not
  installed resolves to `Resolution::Unavailable { owner }` (status
  `unavailable`). Legacy ids are not affected (§2.3).
  Nothing is deleted or rewritten; enabling the module again makes the same
  references resolve.
- The registry never stores values: every resolution asks the owner's
  source again, so a module value is never cached nor persisted as a user
  value.

### 2.3 Aliases and deprecation

`ALIASES` in the registry maps every legacy id to its canonical id, with the
owning module and the version that deprecated it (0.6.0). A legacy
reference keeps its spelling (the `VariableRef` remembers it, so saving a
profile never rewrites it), keeps its static type for validation, and
resolves exactly as it did before the registry, whatever the state of the
module that owns its canonical id (`VariableRegistry::resolve_legacy`):
project values from the context through the core, Work Session values from
the live session through the Work Session module's source even while that
module is disabled, and no value without it. Only canonical ids follow
their module's availability. The picker inserts canonical ids and
hides deprecated entries; the catalog lists them flagged `deprecated` so
older apps keep seeing them.

Deprecated ids (kept indefinitely; removal would need an explicit,
reversible migration of stored references, which is out of scope):

- `system.project.current.id`, `.key`, `.name` → `projects.current.…`
- `system.work.session.id`, `.state`, `.startedAt`, `.elapsedSeconds`,
  `.projectId`, `.canFinish` → `workSession.…`

### 2.4 One resolver

`ContextResolver` resolves every root for actions, workflows, key bindings,
parameter overrides, picker previews and runs:

- `steps.` from the run's outputs;
- `shared.` through the injected shared resolver (store snapshot,
  Project → Profile → Global precedence unchanged);
- `system.`, `workflow.` and module roots through the `VariableRegistry`
  with a `VariableEnvironment { context, profile, os, live }`.

The context is the explicit `ResolutionContext`: active profile, project
(and its source), device and workflow run. Live values that are not part of
the context (the work session) come from `LiveState`, which the agent
implements over its session snapshot. Types are preserved: a source that
answers with another type than declared provides nothing, and whole-value
references keep their native type (`bind`), with no implicit string
conversion for typed parameters.

**Device slot.** The device in the context is read only through the small
`ContextDevice` trait (`VariableEnvironment::device`: `reference_id()`,
`display_name()`). It is implemented by the generic
`deckpilot_device::DeviceDescriptor`, which the live state supplies
(`LiveState::device`): in the core the model catalog describes the device
(`ModelCatalog::describe`; `current_descriptor` does so for the
`DeviceContext::current_device()` of a run), and in the agent the
`VariableStore` holds what `Agent::device_descriptor` gives, with the
driver, updated whenever a device connects or disconnects. A device nobody
can describe falls back to its bare `DeviceId`. `system.device.current.id`
keeps its value, `model#serial`.

### 2.5 IPC (protocol 9)

- `CatalogEntry` gains `owner`, `readOnly` and `deprecated`; groups gain
  `module`; `ReferenceStatus` gains `unavailable`; `ReferenceAvailability`
  gains `unavailable`.
- For clients before protocol 9, `Response::for_protocol` removes entries
  with module roots (they cannot parse them), maps `unavailable` statuses to
  `permissionDenied` and `unavailable` availability to `notInContext`.
  Legacy alias entries stay in the list, so an older app sees exactly the
  ids it knew.
- `sharedVariablesChanged` names both the canonical and the legacy ids of
  Work Session variables that changed; module enable/disable announces the
  module's ids.

### 2.6 Catalog UI and picker

- One **Variables** page replaces the Shared Variables and System Variables
  sections: one table with search; owner, type and scope filters; current
  value (only when not sensitive, redacted otherwise); description; owner;
  read-only/editable; copy reference; create, edit and delete for user
  variables only, under the existing reference-safety rules. No tabs.
- The picker uses the same catalog: name, owner, description, type, current
  value when safe, availability; inserts `{{ canonical.id }}`.
- Implementation: `apps/desktop/src/pages/variables-table.tsx` (the table,
  inside the settings page `pages/shared-variables.tsx`, which keeps the
  shared-variable editor) and `apps/desktop/src/lib/variable-catalog.ts`
  (rows, filters, owners, the app's copy of the aliases for agents before
  protocol 9). Legacy ids are folded into their canonical row ("also
  readable as …"). Sensitive values are never requested for the table.

### 2.7 Live updates

Project, profile and work session changes already reach the agent's
service loop, which re-renders dependent key faces (`after_change`, work
session faces) and emits `projectChanged`, `profileChanged` and
`sharedVariablesChanged`. Pickers and the Variables page refresh previews
on those events; there is no polling.

### 2.8 Structured values and member access

Plugin API 1.12, IPC protocol 18. Some values are not one scalar but an
object of a closed, versioned list of **shapes** (`Shape`, today only
`repository`), or an ordered **map** of such objects by key:

| Type | Written | Example |
| ---- | ------- | ------- |
| `object<shape>` | `{"type": "object", "shape": "repository"}` | `projects.current.repository`: the project's default repository |
| `map<shape>` | `{"type": "map", "of": "repository"}` | `projects.current.repositories`: every repository of the project, by key, in the user's order |

**Member access.** A reference reads one scalar member of a structured
variable: `{{ projects.current.repository.name }}`,
`{{ projects.current.repositories.cms.path }}`,
`{{ projects.current.repositories.cms.remote.defaultBranch }}`. A member path
has 1 to 3 segments (a map key, a group such as `remote`, a member), so a
reference has at most 9 segments (6 for the id). The parser keeps the whole
dotted text; where the id ends is decided by the registry
(`VariableRegistry::split_member`): at the longest prefix that names a
declared structured variable, or a legacy spelling of one. A reference
whose id is declared, or that names nothing structured, resolves exactly as
before; text of 7 to 9 segments, which was not a reference before, now
parses and, naming nothing, resolves as unknown.

**Members of `repository`** (`Shape::members()`):

| Member | Type | On keys | Notes |
| ------ | ---- | ------- | ----- |
| `id` | string | never | an identifier |
| `key` | string | yes | the map key, `cms` |
| `name` | string | yes | |
| `description` | string or null | yes | |
| `isDefault` | boolean | yes | |
| `path` | string or null | **never** (OD-4) | this computer's folder, the binding of the repository's resource; `null` for every reader that may not see paths (below) |
| `exists` | boolean | yes | whether that folder exists |
| `remote.name`, `remote.url`, `remote.defaultBranch` | string or null | `name` and `defaultBranch` yes, `url` never | the primary remote (decision K1) |

A Git Providers connection id and the provider's repository id are never
members (decision K4). A group (`remote`) is read member by member; the group
itself is an unknown member.

**The default repository** (decision OD-3): `projects.current.repository` is
the repository marked as default; without one, the only repository when
there is exactly one; otherwise it has no value (`noDefaultRepository`,
`-` on a key). Never the first by position: reordering must not change what
a key does.

**Never text.** `render_text` gives nothing for structured values: they are
never interpolated into a template, a title or a text parameter (a type
mismatch). A whole value fills only a `Code` parameter of language `json`
(serialized; a map as an object by key) and graph nodes that take JSON.
Shared variables cannot be structured (`DefinitionIssue::Structured`), and a
plugin cannot publish a structured value (`variables.publish` refuses it):
the shape is always the host's statement.

**Resolution.** Missing values have reasons of their own:
`unknownMember`, `repositoryNotInProject` (a key the context's project does
not have; a key of another project is not in this one) and
`noDefaultRepository`. Profile isolation is unchanged: a context of another
profile, or a project of another profile, is `otherProfile` before anything
is read.

**Private paths.** Whoever reads a value tells whether it may see this
computer's folders (`VariableEnvironment::private_paths`):

- the configuration app's previews: yes, as Settings › Resources shows them;
- DeckPilot's own actions: yes;
- a plugin's action, through its parameters or graph mappings into them:
  only with `resources:paths` (`ActionRegistry::reads_paths`), otherwise
  `path` is `null` (a template that uses it fails as missing);
- key titles: never.

The host source leaves the folder out for such a reader, and the registry
removes it again from whatever a source returns (`read_member`, `checked`),
so no source can hand a path to a reader that may not see it.

*Limit of the plugin gate.* It applies to the variable references a
plugin action's own parameters and mappings make. A value is not marked
with where it came from: once a node that may see paths has read a folder
(a Set variable node, DeckPilot's own action, a plugin with
`resources:paths`), it is that node's output, plain text, and a plugin
without `resources:paths` whose mapping names it receives it:
`{{ steps.<node>.output.… }}`, a workflow variable
(`{{ steps.vars.output.… }}`) or a workflow's input
(`{{ steps.start.output.… }}`) that its caller filled from a path. Only a
mapping the workflow's author wrote passes it on; nothing reaches such a
plugin without one. Pinned by
`a_folder_passed_on_by_an_earlier_node_reaches_a_plugin_without_paths`
(`crates/deckpilot-core/src/graph/tests/references.rs`).

**Filesystem access.** The only one is `exists`, a directory check of the
repository's folder (`FolderProbe`). A read of one member builds only what
it needs (`VariableSource::resolve_member`): the one repository it names,
and its folder is looked at only when `exists` itself is read (or a whole
object or map), so a key that shows a name never waits on an unmounted
network volume.

**Catalog and previews (protocol 18).** `listVariables` lists a structured
variable once, with its shape's `members` and, for a map, the keys of the
context's project's repositories in the user's order (`mapKeys`). The agent
never lists one entry per member: the app composes them
(`expand_members`, in the desktop app's `list_variables` command), one per
member (`….repository.name`) and for a map one per key and member
(`….repositories.cms.name`), each naming its variable (`memberOf`). So a
catalog grows by one key per repository (at most 32 characters, at most
256 repositories) and always fits one IPC message (64 KiB); a search
(`CatalogFilter`) lists a structured variable when it or one of its
members matches, and the app keeps only the matching ones.
`listDisplayVariables` lists a structured variable for its display-safe
members only (`membersOnly`, §3.2); the variable itself is never offered. A
preview of a whole structured value is `structured`: how many objects,
never a member. A client of protocol 17 or older receives structured types
as `json` without members or keys, no members-only entries, a structured
preview as `redacted` JSON, and `noValue` or `unknownVariable` for the new
reasons.

**Legacy spellings.** `projects.current.repositories` and
`projects.current.repository` are module variable aliases of
`plugins.deckpilot.projects.current.repositories` and `….repository`
(`MODULE_VARIABLE_ALIASES`, `moved: false`: no module ever had them). They
are declared by Projects 0.2.0 (`^1.12`) with the host sources
`host:project.repositories` and `host:project.repository`; until it is
bundled they resolve as unavailable with the plugin.

## 3. Variables in key titles

Key titles are **display templates**: plain text with references in the
reference grammar (`{{ projects.current.key }}`, `{{ shared.team.name }}`,
`{{ system.profile.current.name }}`), mixed with text and with each other
(`Projekt: {{ projects.current.key }}`). The older key label placeholders
keep working unchanged: `{profile}`, `{project.key}` and `{project.name}`
are resolved through the engine as `system.profile.current.name`,
`system.project.current.key` and `system.project.current.name`; `{page.…}`
stays the layout's (`docs/key-text.md`). The title is stored as typed; nothing
is migrated or rewritten.

### 3.1 One function

`deckpilot_core::variables::resolve_display_template(environment, template)`
returns the text, the unresolved references (name and reason, never a
value), the positions of their markers (`markers`, UTF-16 offsets), text
between `{{ }}` that is not a reference (`invalid`, kept as typed) and the
dependencies of the text. The environment is the engine's
(`Engine::display_templates`): shared variables from the agent's snapshots,
core and module variables from its registry, in a **display context**: the
profile, its active project, a preview phase, no device, workflow or run.

- The device: `Runtime::display_faces` resolves every title through it while
  drawing a page (`page_faces_with`), and records each text by **binding and
  text slot** (`TextKey`: page, control, trigger, `TextSlot::Title`).
- The editor: `resolveDisplayTemplates` (protocol 17) runs the same function
  in the agent; `preview_layout` asks it for the page's templates and draws
  with its answers (`ControlFace.titleDisplay` carries them), so the Canvas
  and the device agree. Without an agent the app resolves with the core alone
  (`core_display`): the older placeholders, profile, project and platform.

The function is not tied to the title: further key labels add `TextSlot`
variants, each resolved by the same call and tracked under its own key.

### 3.2 Display-safe sources

Only these resolve on a key (`display_safe`, `display_listed` for the
picker, `listDisplayVariables`):

| Source | Resolves |
| ------ | -------- |
| `system.profile.current.name`, `system.project.current.key`/`.name`, `system.platform` | yes |
| Module and plugin variables | when declared non-sensitive, of string, integer or boolean type, with a machine, profile or project scope and no requirement but a project, while the module or plugin is enabled |
| Shared variables | when not sensitive and of string, integer or boolean type |
| Sensitive values (declared, or marked so by their source when resolved), secret references (credentials), `steps.…`, `workflow.…`, `system.device.…`, the work session | never |
| Ids: `system.project.current.id`, `projects.current.id`, `plugins.deckpilot.projects.current.id`, any plugin variable with a `host:project.id` source (`VariableDeclaration::identifier`), `workSession.id`/`.projectId` | never |
| Members of a structured variable (§2.8): `….repository.name`, `….repositories.cms.key`, `….remote.defaultBranch`, `….exists` | when the shape marks them display-safe, under the rules of their variable above |
| A whole structured value (`projects.current.repositories`, `….repository`, `….repositories.cms`), a repository's `id`, `path` or `remote.url` | never (`notDisplayable`); a folder path never reaches a key (OD-4) |

A refused reference is never resolved where its declaration already says so;
a shared variable is dropped as soon as its resolution says it is sensitive.
Profile isolation is the engine's: values of another profile are missing.

Every title is a template, whoever wrote it: the key's or the action's
label, a layout's name on *Switch layout*, an action's name, and a plugin's
suggested title (`title.default` of its manifest). A plugin can therefore
show an allow-listed value on a key through its suggested title, exactly as
a user could; it never reads the result back (titles are drawn, not
returned to plugins), so nothing leaks to it.

**Length.** A title without references is at most 32 characters, as before.
A title with references may be up to 160 characters as stored
(`MAX_TITLE_TEMPLATE_CHARS`, so plugin references fit); what the key shows is
cut to 32 characters (`…`). DeckPilot 0.12 and 0.13 open layouts with such
titles; they check the length only when saving, so they refuse to save one, and
to duplicate an action whose label is one, until the title is shortened.

**Page placeholders.** `{page.number}`, `{page.count}` and `{page.name}` are
expanded in the template before references are resolved, so a value that
reads `{page.name}` stays as it is.

### 3.3 Unresolved references

A reference without a displayable value shows `-` (`UNRESOLVED_MARKER`),
the mark the older placeholders always showed without a project: never a
stale value, never an error, and the template is kept. The editor tells a
marker from a value that reads `-` by `markers`, and warns with
`unresolved` (non-destructive).

### 3.4 Updates

Without polling. Profile and project changes redraw every device
(`after_change`); a `sharedVariablesChanged` (a shared value after a reload,
a plugin's published value, a module or plugin enabled or disabled) redraws
only the devices showing a text that depends on one of the named keys
(`TitleDependencies`, `DisplayDependency::changed_by`). Such redraws are coalesced per
device: the first change schedules one 50 ms later, further changes ride
along, a device is redrawn for variables at most 20 times a second, and the
last value is always drawn; input, actions and page changes never wait for
them (`service/display_redraws.rs`). A plugin that publishes the value it
already had announces nothing. A device is sent only the keys whose face
changed. Plugin state titles keep their precedence
(`docs/key-text.md`, "Title precedence"): a template the user typed on a
plugin action's key is the user's title.

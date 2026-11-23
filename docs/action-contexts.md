# Action contexts

An action type can say **where it makes sense**: on a key, as a step of a
Multi Action, as a node of a workflow, as a key's live display, or on a
gesture. These are its *contexts*. The core type is
`deckpilot_core::ActionContexts` (`crates/deckpilot-core/src/contexts.rs`);
`ActionType.contexts` holds it.

Contexts are **advisory in this release**. They are metadata that the app
can use to offer each action where it belongs. Nothing that worked before
stops working:

- Saving a layout or a workflow **reports** a use outside its action's
  contexts and **refuses nothing**.
- Running one **logs a warning** (see "Running" below for when) and **still runs**.
- Nothing stored is removed, converted or rewritten.

Contexts are not permissions and not a security boundary.

Contexts belong to action types. A key or gesture that starts a workflow
(`BindingTarget::Workflow`) is not an action type and has no contexts: it
goes on any key primary, dial or touch trigger and on gestures. What a
save refuses and how a run is re-checked for workflow targets is in
[data-model.md](data-model.md#workflows-on-keys).

## The five contexts

| Context | Identifier | A use is in it when |
| --- | --- | --- |
| Key | `key` | It is bound to a key's, dial's or touch zone's primary input: a press, release or click, a rotation, a touch |
| Gesture | `gesture` | It is bound to a double click or a long press |
| Display | `display` | Its key's face is the point: a live state. A type with `display` but without `key` is display-only, and pressing its key does nothing |
| Sequence | `sequence` | It is a step of a Multi Action, which is a workflow without a graph |
| Workflow | `workflow` | It is an action node of a graph workflow |

The context of a use always comes from data that already exists: a
binding's trigger, a step, a node. It is never derived from an action's
name, category or id.

Two more rules apply:

- A key's primary input also accepts a display-only type.
- Runs that the app starts have no context and are never checked. These
  are the editor's Run button, `runAction` and a plugin's Test run.

Contexts are independent of [controls](device-controls.md). Controls
decide which control kind and input an action can be bound to. Contexts
decide whether the use makes sense. A binding is checked against both.

### Without a declaration

A type that declares no contexts gets all five, exactly as before contexts
existed. This applies to every plugin action, to stored definitions and to
types from older agents. The field is optional on the wire: it is left out
when absent, and older apps and agents ignore it.

## Built-in declarations

Every built-in action type declares its contexts. The contract test
`every_builtin_action_declares_its_contexts`
(`crates/deckpilot-modules/tests/contracts.rs`) enforces this table:

| Actions | Contexts | Why |
| --- | --- | --- |
| `general.log` | sequence, workflow | Its message only reaches the run's history |
| `general.wait` (deprecated), `general.delay` | sequence, workflow | On a key a built-in runs in place, so the wait blocks and cannot be cancelled |
| `deck.next-page`, `deck.previous-page`, `deck.switch-profile`, `deck.brightness`, `deck.random`, `deck.switch-layout`, `deck.previous-layout` | key, gesture | A deck request is applied only for a key or a gesture. In any workflow it fails with `not-from-a-key` |
| `deck.multi-switch` | key, gesture, display | As above. Its face shows the current state |
| `deck.page-indicator` | display | Its face shows the page; its press does nothing |
| `system.open-app`, `system.open-url`, `system.open-file`, `system.open-directory`, `system.run-command`, `http.request`, `notifications.show` | key, gesture, sequence, workflow | General purpose |
| `system.process-status`, `http.health-check` | sequence, workflow | Queries: the result drives a step condition or later nodes |
| `system.cpu`, `system.memory`, `system.network`, `system.disk` | display, workflow | A live reading on a key, or a value for a workflow |
| `notifications.workflow-summary` | workflow | Only a graph run has a summary |

The test also checks these rules for each declaration:

- At least one context is declared.
- `gesture` needs a double click or long press in the type's controls.
- `key` needs a press, release, click, rotation or touch input in the
  type's controls.
- `display` needs a live face: a `state_policy`, a `decorate_face` that
  changes the face, or a face the agent draws itself (`deck.multi-switch`).

The declarations leave every type's `controls` unchanged.

### Wait is deprecated

`general.wait` is marked `deprecated` in favour of `general.delay`, which
does the same in milliseconds. In a graph, the native Delay node does the
same job.

The id and the handler stay unchanged, so saved Wait actions, steps, nodes
and keys keep working. Deprecated types are not offered for new actions,
which is the existing behaviour for every deprecated type
([modules](modules.md)). Nothing is converted automatically.

## Legacy aliases

A legacy built-in id that a first-party plugin now serves takes the
plugin's definition, so it **inherits the plugin's contexts**. A plugin
action without a declaration has all five.

`alias::context_incompatibilities(builtin, target)` lists the contexts the
built-in declares that the plugin's action does not. `ModuleRegistry::
action_aliases()` reports them as `AliasStatus.context_issues`. This rule is
**reported only**: unlike `alias::incompatibilities`, it never changes the
routing. A plugin that narrows its contexts therefore never sends a legacy
type back to the built-in and never makes it unavailable.

## Saving: reported, never refused

`DeviceLayout::context_issues_for_save` and `Workflow::context_issues`
report each use outside its action's contexts as a `ContextIssue`:

- `site`: a binding (page, control, trigger), a step (index from 0) or a
  node (id).
- `action`: the action instance.
- `reason`: `IncompatibilityReason::ContextUnsupported { context, supported }`.
  It serializes as
  `{"type":"contextUnsupported","context":"key","supported":["sequence","workflow"]}`.
- `stored`: whether the same use was saved before. A binding counts as the
  same when its page, control, trigger and action match. A step counts when
  the same action is a step of the saved Multi Action, at the same index or
  elsewhere, counted as a multiset. A node counts when its id and action
  match.

The `stored` flag is there for a future policy that refuses new
incompatible uses while keeping stored ones. No such policy exists yet.

Converting a Multi Action to a graph changes how its uses are compared.
Before the conversion they are steps; afterwards they are nodes, matched
by node id against a saved graph. The first save of the converted
workflow has no saved graph to match, so every node is reported with
`stored: false`, even when the same action was a stored step. This only
matters for a future refusal policy, which would have to treat a
conversion as keeping its stored uses.

`save_layout` and `save_workflow` return these as `contextIssues`, which is
left out when empty. They are not part of a layout's `incompatible` list,
because those bindings are kept but not run, while these still run.

The following get no verdict, and the existing unavailable path applies:

- an action that is missing;
- a type that is unavailable, for example because its plugin is missing
  or disabled.

A graph workflow's `steps` are kept only for reference, so they are not
checked as steps.

## Running: warn, then run

Every action run passes through `Engine::execute_with_outputs`, which
checks the use's `Invocation.context`. The context is set as follows:

| Run | Context |
| --- | --- |
| A key's press, release, click, rotation or touch (device runtime) | `key` |
| A double click or long press | `gesture` |
| A Random Action's pick, a Multi Action Switch's state | `key` |
| A Multi Action's steps | `sequence` |
| A graph workflow's action nodes | `workflow` |
| The app (Run, `runAction`), a plugin's Test run | none: not checked |

A use outside the type's contexts **still runs**. The engine reports it to
its `ContextWarnings` sink, and the agent logs it as a warning, the first
time a given combination of **context, workflow and action instance**
runs. The workflow is `None` for keys and gestures. This key is coarser
than a single binding, step or node:

- Two keys bound to the same action instance in the same context (for
  example two presses) share one warning.
- Two steps of one Multi Action that use the same instance share one
  warning, and so do two nodes of one graph.
- The same instance on a key, on a gesture and in each workflow is
  warned about separately.

The set of uses already reported belongs to the engine, and the agent
builds a new engine whenever it reloads the configuration, for example
after every save in the app. So the warning comes again once after each
reload. That is accepted for now: it only adds log lines.

A deck action in a workflow still fails exactly as before, with
`not-from-a-key`.

## Planned

- **Plugin manifest field.** The planned field is
  `contributes.actions[].contexts`: an optional list of the five
  identifiers, with the same rules as above. Without it a plugin keeps all
  five. The field **needs plugin API 1.11 or later**, and 1.11 can ship
  only **after 1.10**, which is reserved for gesture presets. Shipping 1.11
  first would let a 1.11 agent accept a `^1.10` manifest whose
  `gesturePresets` it does not know. Until then no manifest declares
  contexts, the agent's plugin API stays where it is, and every plugin
  action has all five.
- **Refusing new incompatible uses.** A later release may refuse a *new*
  use outside the contexts while keeping stored ones, using the `stored`
  classification above. Such a policy needs an approved decision first.
- **The app.** The Action Library, gesture cards, step editor and workflow
  builder will offer each type only where it makes sense, with a "Show
  all" switch.
- **Detached built-ins.** Long-running built-ins (`system.run-command`,
  `http.request`) run in place on a key today. Running them detached is a
  separate change.

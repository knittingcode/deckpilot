# Compatibility between app and agent versions

The configuration app and the background agent ship together and normally
run the same version. They can still differ for a while: right after an
update the previous agent may still run, an older app may be started
against a newer agent, and an agent may restart under an open app. This
page says what happens then and how it is tested.

## Versions involved

| Build | App/agent version | IPC protocol | Database schema |
| ----- | ----------------- | ------------ | --------------- |
| Tag `v0.5.0-2` (signed draft test build, never published) | 0.5.0-2 | 4 | 5 |
| Releases 0.6.0, 0.7.0, 0.8.0 and 0.8.1 | 0.6.0, 0.7.0, 0.8.0, 0.8.1 | 10 | 9 |
| Release 0.9.0 | 0.9.0 | 11 | 10 |
| Release 0.10.0 | 0.10.0 | 16 | 13 |
| Release 0.11.0 | 0.11.0 | 16 | 13 |
| Release 0.12.0 | 0.12.0 | 16 | 13 |
| Release 0.13.0 | 0.13.0 | 16 | 13 |
| Release 0.14.0 | 0.14.0 | 17 | 13 |
| Release 0.15.0 | 0.15.0 | 17 | 13 |
| Current main | see `Cargo.toml` | 20 | 16 |

Besides these, 0.10.0 provides plugin API 1.9 (0.9.0 provides 1.4): the
permissions `tools`, `resources`, `projectContext` and `hostViews`, an
action's `requiresPermissions` and visual assets, accepted only from
DeckPilot's own bundled plugins
([module-migration.md](marketplace/module-migration.md) §7, §11); older
agents refuse a manifest that uses them. Git, Docker, Workspace, Projects,
GitHub, GitLab and Jira are no longer built in: their bundled plugins take
over, a 0.9.x configuration migrates through static descriptors after an
encrypted backup (`deckpilot-pre-plugin-extraction-*.db`), and keys whose
plugin is not active yet stay in the layouts as unavailable. The plugin
store (`plugins.db`) goes from schema 5 to 6 (host-held tool settings); a
0.9.x agent refuses the upgraded file and runs without plugins. The
configuration database goes from schema 10 to 13 (folder controls, folder
titles), each step after an encrypted backup.

0.11.0 changes neither the protocol nor a schema: plugin API 1.9, plugin
store schema 6, and the same bundled plugins as 0.10.0. It adds the
first-party publisher keys that official plugins from a registry must be
signed with ([publisher-signature.md](marketplace/publisher-signature.md));
the agent keeps its floors in `<data>/plugin-official/floors.json`, which
an older agent does not read.

0.12.0 changes neither the protocol nor a schema either: plugin API 1.9,
plugin store schema 6. Its nine bundled plugins are new versions, taken
from their signed releases in `knittingcode/deckpilot-plugins` (GitHub
0.2.1; GitLab, Jira and Notifications 0.1.2; the others 0.1.1), with the
same permissions as in 0.11.0. A 0.11.0 or 0.10.0 installation replaces
its bundled copies with them when 0.12.0 starts.

0.13.0 changes neither the protocol nor a schema: plugin API 1.9, plugin
store schema 6, and the same nine bundled plugins (same versions and
bytes) as 0.12.0. It adds the IPC feature `storefront` without a protocol
bump: `getStorefront`, `getRegistryListing` and `getRegistryMedia`, which
the app sends only to an agent that announces the feature (against an
older agent the app's storefront commands answer `null`). The agent caches
a verified storefront under `<state>/storefront/` and its media under
`<state>/media/`; an older agent does not read them.

Current main (after 0.15.0) moves the configuration database to schema
14: projects get their repositories (`docs/data-model.md`,
"Repositories"). The migration is additive. It adds two columns to
`projects` and stores, for every project with the `repository` role, the
repository that role stands for. Roles, resources and folder bindings are
not rewritten. As every upgrade, it runs after an encrypted backup next to
the database (`backups/deckpilot-schema13-*.db`). **0.15.x and older
refuse a schema 14 database** with `UnsupportedSchema` (found 14,
supported 13) and change nothing. The way back to 0.15.x is that
pre-migration backup: restore it in place of `deckpilot.db`, without its
`-wal` and `-shm` files. Changes made after the upgrade are not in the
backup. Exports with repositories other than the one the role stands for
are payload version 5, which 0.14.x refuses; other exports keep their
version.

IPC protocol 19 adds Calls (capability `calls`,
`docs/calls.md`): `getCallsStatus`, `setCallsOverlay`,
`restoreCallsLayout` and the `callsChanged` event. Older clients never send
these requests: they are refused locally and by the agent. Older clients
receive no `callsChanged` events, and an older agent leaves the app's Calls
settings hidden. Only desktop clients receive `callsChanged`; that is
hygiene, since the IPC token is the access boundary. Protocol 18, the
step before it, is structured variables.

Current main then moves it to schema 15: the managed layouts of the Calls
plugin (`docs/calls.md`, "Managed layouts"). The migration is additive: one
new, empty table (`managed_layouts`); no layout is adopted or rewritten and
no other table changes. It runs after the same encrypted backup
(`backups/deckpilot-schema14-*.db`, or `-schema13-` when 14 and 15 run in
one upgrade). **0.15.x and older refuse a schema 15 database** with
`UnsupportedSchema` (found 15, supported 13), and a build of schema 14 does
too (supported 14); they change nothing. The way back is the pre-migration
backup, as above: restore it in place of `deckpilot.db`, without its `-wal`
and `-shm` files.

Current main then moves the configuration database to schema **16**,
after 15: the scripts of a profile (`docs/data-model.md`, "Scripts"). The
migration is additive, two new, empty tables: `scripts` (portable, with its
profile) and `script_trust` (machine-local: the trusted source hash and the
script's grants, never exported). Nothing existing is rewritten. It runs
after an encrypted backup (`backups/deckpilot-schema15-*.db`, or the
older schema's when 15 and 16 run in one upgrade). **Older builds refuse a
schema 16 database** with `UnsupportedSchema` and change nothing; the way
back is that backup, as above.

0.14.0 speaks protocol 17 and changes no schema: database schema 13,
plugin API 1.9, plugin store schema 6, and the same nine bundled plugins
(same versions and bytes) as 0.13.0. Protocol 17 adds two capabilities:
`displayTemplates` (variables in key titles: `listDisplayVariables` and
`resolveDisplayTemplates`) and `keyLabels` (key labels in layouts and
faces, optional fields older peers ignore). With a protocol 16 agent the
app shows titles resolved by the core alone (the older placeholders) and
the device draws no labels; a protocol 16 app with a 0.14.0 agent works as
before, and its keys show resolved titles. The stored data an older
version may meet: titles with references of up to 160 characters (0.12
and 0.13 refuse to save them until they are shortened), labels (an older
agent ignores them, an older app drops them when it saves the layout),
exports with labels as payload version 4 (older versions refuse them),
actions saved before they are configured (an older app refuses edits to
them), and the new action type `system.close-app` (kept as unavailable by
older versions). Workflow references, the guarded delete and the new
faces change no stored data.

0.15.0 changes neither the protocol nor a schema: IPC protocol 17,
database schema 13, plugin API 1.9, plugin store schema 6, export payload
versions unchanged (4 with labels), and the same nine bundled plugins
(same versions and bytes) as 0.14.0. The live preview's navigation gains
the optional `folderCountdown` (the Automatic exit countdown the Back key
shows), which a 0.14.0 app ignores and a 0.14.0 agent never sends (the
editor's Canvas then draws the Back key without it). A folder's existing
`autoReturn` keeps its stored form; what changes is when the agent
follows it: a 0.15.0 agent leaves the folder once the run its key started
is over, an older agent when the key is handled. The missing-plugin face,
the Marketplace and the moved settings pages change no stored data.

Before it, protocol 16: the Device Editor's live preview
([editor-navigation-and-simulation.md](editor-navigation-and-simulation.md#3-live-preview-and-navigation-sync)).
Protocol 16 adds the `livePreview` capability: `beginLivePreview`,
`applyLivePreview`, `navigateLivePreview` and `endLivePreview`, with which
the app's editor has one device show and run its unsaved layout (kept in
the agent's memory only, never saved) and the two follow each other's
navigation; `livePreview` messages go only to the connection that owns the
session. `inspectMockDevice` reports a simulated device's frames and
navigation and, like `simulateInput`, is refused without mock devices.
`DeviceInfo` carries the optional `pageId`, which older apps ignore. The
client never sends the new requests to an agent below protocol 16 (refused
locally with `Unsupported`), and the agent refuses them from a connection of
an older protocol; the app then says that changes reach the device when
saved, as before. Nothing an older app or agent sends or receives changes,
and no schema changes.

Before it, protocol 15: restoring DeckPilot's own plugins.
Protocol 15 adds the `restoreBundled` capability (and the plugin runtime
feature `restoreBundled`): the `restoreBundledPlugin` request installs a
bundled first-party plugin the user uninstalled again from the
application bundle, verified against the pins compiled into the agent (the
user's tombstone is cleared), or reinstalls one whose installed copy is
broken, and answers the plugin's details; `listKnownPlugins` marks the
removed plugins the agent can restore with the optional `restorable`,
which older apps ignore. The client never sends the request to an agent
below protocol 15 (a raw request is refused locally with `Unsupported`,
which the app reports as `agentTooOld`), and the agent refuses it from a
connection of an older protocol. Nothing an older app or agent sends or
receives changes, and no schema changes: the restore uses the store's
existing install, uninstall and tombstone records.

Before it, protocol 14 and plugin API 1.7: the Device
Editor's interactive preview
([editor-navigation-and-simulation.md](editor-navigation-and-simulation.md)).
Protocol 14 adds the `simulateInput` capability: the `simulatePress`
request, with which the app presses a control of a connected device through
the agent's own gesture engine and dispatch (answered `simulatedPress`).
The client never sends it to an agent below protocol 14 (a raw request is
refused locally with `Unsupported`, which the app reports as the refusal
`unsupported`), and the agent refuses it from a connection of an older
protocol; nothing an older app or agent sends or receives changes. The
older, mock-only `simulateInput` request is unchanged. Workflows a
simulated press starts are recorded with the execution trigger `simulated`
and the workflow cause `simulated`, which older readers see as `other`
(both enums have a catch-all). Action types may carry the optional
`editorNavigation` and preview faces the optional `navigation`, fields older
apps ignore; neither needs the new protocol. Plugin API 1.7 adds the
manifest's `contributes.actions[].editorNavigation` (no ABI or SDK
change): a manifest with it needs `^1.7` (the validator refuses it below),
so an agent of plugin API 1.6 reports such a version `incompatible`;
manifests without it are unchanged.

Key labels (0.14.0): a key's appearance may carry `labels`, an optional
list that older agents ignore (the key draws without them) and an older app
drops when it saves the layout; no document version or migration. Exports
that contain labels are written as payload version 4, which older versions
refuse rather than import without labels; exports without labels keep
version 2 or 3. IPC: the capability `keyLabels` (protocol 17, with
`displayTemplates`); key faces carry `labels` and previews `labelBoxes` only
when a key has labels ([key-text.md](key-text.md#labels)).

Key title appearance (unreleased): a key's appearance may carry
`fontFamily`, `bold`, `italic` and `underline`, optional fields that older
agents and apps ignore (an older app drops them when it saves the layout);
no document version, migration or IPC protocol change. Action types may
carry the optional `title` (older apps ignore it). Plugin API 1.8 adds the
manifest's `contributes.actions[].title` and `contributes.visualAssets`
(no ABI or SDK change): a manifest with either needs `^1.8`, so an agent of
plugin API 1.7 reports such a version `incompatible`; manifests without
them are unchanged ([key-text.md](key-text.md#plugin-titles),
[marketplace/sdk/manifest.md](marketplace/sdk/manifest.md#contributesvisualassets)).

Shared visual assets (unreleased,
[key-text.md](key-text.md#icons-and-backgrounds)): a key's `icon.id` may be
a library asset id (`deckpilot.<name>`, `<plugin>.<id>`) besides a profile
image's UUID, and its `background` may be `asset:<ref>` besides `#rrggbb`
and `transparent`. Stored values of before read unchanged; no document
version and no migration. An older app or agent refuses a layout that uses
one of the new values (as for any unknown value), so such layouts need this
version on both sides. No IPC protocol change (it stays 16): plugin
summaries carry the optional `visualAssets` (older apps ignore it), and
`getPluginIcon` the optional `asset`, which the app sends only to an agent
whose plugin runtime lists the feature `visualAssets` (an older agent would
answer with the plugin's own icon).

Plugin API 1.9 (unreleased,
[marketplace/module-migration.md §7](marketplace/module-migration.md#plugin-api-19-security-reviews-2026-11-20)):
additive. The manifest kinds `tools`, `resources`, `projectContext` and
`hostViews`, `openUrl.fromParams`, `resource` parameters, the view kinds
`workflows` and `projects`, and `contributes.statusItems` and
`contributes.dashboardCards` validate only with `^1.9` and only for a
first-party plugin shipped inside the app; every other manifest is refused
them, and a `^1.8` manifest without them validates exactly as before. The
capability operations `resource.resolve`, `resource.open`, `tool.run`,
`url.openParam`, `project.current`, `project.list`, `project.select`,
`project.cycle` and `profile.read` are compatible additions within ABI 1
(a guest that does not use them never sends them; an older agent refuses
them as unknown). This agent reports plugin API 1.9 and carries out all of
them; an agent of 1.8 reports a `^1.9` version `incompatible`. The SDK's
`LATEST_API_LEVEL` is 1.9. An older app ignores a navigation entry of an
unknown view kind. A manifest action's `node.changesContext` (a first-party
plugin with `projectContext.select` only) is another `^1.9` field that an
older agent refuses as unknown; a `resource.resolve` role is the user's
role name (up to 1,024 characters, not blank), no longer a parameter key.
No IPC protocol or document change.

Plugin API 1.10 (unreleased,
[gestures-and-work-session.md §3.4](gestures-and-work-session.md#34-gesture-presets)):
additive. `contributes.gesturePresets` validates only with `^1.10`; a
`^1.9` manifest without it validates exactly as before, and an agent of
1.9 (0.15.0 and older) reports a `^1.10` version `incompatible`. Presets
are data only: no capability operation, ABI or permission is added, and
nothing of a preset is stored, so no document, schema or migration
changes. `ModuleInfo.gesturePresets` and `PluginSummary.gesturePresets`
are optional fields, left out when empty, that an older app ignores; an
older agent sends neither, and the app then lists no presets. No IPC
protocol change. The SDK's `GESTURE_PRESETS_API_LEVEL` is 1.10 (nothing
for the guest).

Plugin API 1.11 (unreleased, [action-contexts.md](action-contexts.md#plugin-actions)):
additive. `contributes.actions[].contexts` validates only with `^1.11`;
a manifest without it validates exactly as before and its actions keep
all five contexts. An agent of 1.10 or older reports a `^1.11` version
`incompatible`. The declaration reaches the app inside the action type
(`ActionType.contexts`, the optional field 0.14.0's built-in contexts
already use), so there is no IPC protocol, schema or
document change; contexts stay advisory (reported on save, warned on
run, never refused). The SDK's `LATEST_API_LEVEL` is 1.11
(`ACTION_CONTEXTS_API_LEVEL`, `Action::contexts`).

Plugin API 1.12 (unreleased, [variables.md §2.8](variables.md#28-structured-values-and-member-access)):
additive. Structured variable types (`valueType` `{"type": "object",
"shape": "repository"}` and `{"type": "map", "of": "repository"}`) and the
host sources `host:project.repositories` and `host:project.repository`
validate only with `^1.12`, only with exactly their source's type, only for
a bundled first-party plugin with `projectContext` (privileged constructs,
so the reviewed privileged policy must list them before a plugin may use
them), and a structured type only with a host source. A `^1.9` manifest
without them validates exactly as before. The validator knows 1.12; the
agent's `PLUGIN_API` stays 1.11 until the release that bundles Projects
0.2.0, so an agent reports a `^1.12` version `incompatible` until then.
Scalar references, templates and stored `{{ … }}` texts resolve exactly as
before; text of 7 to 9 segments under a module or plugin root, which was not
a reference, now parses as a member reference and resolves as unknown. No
database, document or export change.

IPC protocol 20 (unreleased; 19 is reserved for the Calls event):
plugin connections (capability `pluginConnections`, plugin API 1.14):
`listPluginConnections`, `createPluginConnection`,
`setPluginConnectionSecret`, `updatePluginConnection`,
`testPluginConnection` and `removePluginConnection`. A client of an older
protocol never sends them (refused locally) and the agent refuses them from
such a connection, so a token never reaches an agent that would not keep
it; nothing older clients receive changes. The plugin store
(`plugins.db`, separate from the configuration database) goes from schema 6
to 7: `plugin_connections` and `plugin_connection_secrets`, additive. An
older agent refuses a schema 7 `plugins.db` with `UnsupportedSchema` before
writing anything and runs without plugins; going back means restoring the
`plugins-schema6-…` backup written before the upgrade. The configuration
database, documents and exports do not change: connections and their
tokens are machine-local and never exported. The agent's `PLUGIN_API`
stays 1.11, so `^1.14` plugins are still reported `incompatible`.
Also in protocol 20, with the plugin runtime feature `repositoryRemotes`:
`listRepositoryRemotes {profile, project, repository}` answers the git
remotes of that repository's bound folder (`repositoryRemotes`, at most 16,
URLs without credentials), read by the agent with `git remote -v`; the app
asks only an agent that lists the feature, and sends ids, never a path.

IPC protocol 18 (unreleased): structured variables (capability
`structuredVariables`). `CatalogEntry.valueType` may be `object` or `map`
with the shape's `members` and, for a map, its keys in the context's
project (`mapKeys`); each structured variable is listed once and the app
composes its member entries (`memberOf`), so a catalog stays within one
message with up to 256 repositories. The title picker's catalog marks a
structured variable listed for its display-safe members only
(`membersOnly`). A preview of a whole structured value is `structured` (a
count, never a member), and the reasons `unknownMember`,
`repositoryNotInProject` and `noDefaultRepository` are new. An agent of 18
answers an older client with structured types as `json` without members
or keys, no members-only entries, a structured preview as `redacted` JSON
and the nearest older reasons
(`Response::for_protocol`); no request changes, so an app of 18 works with
an agent of 17 as before (without repository variables).

Plugin API 1.13 (unreleased, [scripts-bridge.md](scripts-bridge.md)), the
Scripts host bridge, part 1 of 4: the ABI and the host engine. The
operations `script.*`, `project.repositories`, `fs.*`, `process.run`,
`shell.run` and `shell.available` are compatible additions within ABI 1:
a guest that does not use them never sends them, an older host answers
`unsupported-op`, and this agent's broker still refuses all of them to
every plugin (the `scripts` permission and the executor follow). A
`CallError` may carry the optional `script` detail, which the host keeps
only on those operations' replies, so no existing guest receives it.
`GuestLimits` gains the optional `hostCalls` and `polls`, which the
supervisor sends to its own host only (they ship together); without them
a guest keeps its 256 calls per invocation. One rule changes for every
guest: a reply the host gives itself (a malformed request, a call refused
inside `dp_init` or a state call) no longer resets the continuous-CPU
deadline; such replies are immediate, so no guest that waits on the
agent is affected. No frame cap, IPC protocol, schema, document or
export change, and `PLUGIN_API` stays 1.11.

Part 2 adds the `scripts` permission to the broker (the ceiling, for
`deckpilot.scripts` only) and the agent's executor for the run's own
operations, the projects and the file system. No manifest can declare
`scripts` yet (part 4), and the agent keeps no scripts yet (P7), so
nothing changes for an installed plugin. The agent gains the `cap-std` and
`cap-fs-ext` dependencies (Apache-2.0 WITH LLVM-exception, Apache-2.0 or
MIT). No IPC protocol, schema, document or export change.

Part 3 adds the agent's executor for processes (`process.run`,
`shell.run`, `shell.available`) through the tools runner of `tool.run`;
the runner gains stdin and a stream cap per call, which `tool.run` does
not use. No IPC protocol, schema, document or export change.

Part 4, the manifest at plugin API 1.13: the validator knows the
`scripts` permission (`fs`, `process`, `shell`; high tier), the `script`
parameter kind, the `javascript` language of a `code` parameter, the
`scripts` view (a host view, with `hostViews`), and the runtime
constructs `runtime.limits.continuousMs` (250 to 10,000 ms) and
`runtime.hostCalls` (1 to 4,096). Each needs `^1.13`; all but the
language are privileged (first-party bundled plugins only), and the
permission and the runtime constructs are for `deckpilot.scripts` only,
which cannot also declare `network`, `credentials` or `openUrl`. The
plugin store records the `scripts` grant and the agent passes it to the
broker; the supervisor gives `deckpilot.scripts` its deadline and call
budget (with the poll budget always beside it) and no other plugin a
budget. Both schema copies change alike. The agent's `PLUGIN_API` stays
1.11 (P8 moves it to 1.13), so a `^1.13` plugin is `incompatible` until
then; a `script` parameter has no core kind yet (P7) and its action
refuses to load as an action type (`javascript` code is the core's text
`code` kind). No IPC protocol, database, document or
export change.

The plugin store goes from schema 5 to 6 with the extraction's cut-over
(`docs/marketplace/module-migration.md` §7 X13): an additive
`host_settings` table, after the usual encrypted backup in
`plugins/.backups/` (`plugins-schema5-…`). It holds what the host keeps
for itself and never hands a plugin: the built-in `git` and `docker`
modules' executable, context and time limits that `tool.run` uses
(`tools.git`, `tools.docker`), and the journal's note of the encrypted
`deckpilot-pre-plugin-extraction-…` backup of the configuration database
taken before the first extraction cut-over. A 0.9.x agent (store schema 5)
refuses a schema 6 `plugins.db` with `UnsupportedSchema` before writing
anything, reports the plugin store unavailable and runs without plugins:
its built-in `git`, `docker`, `workspace`, `projects`, `github`, `gitlab`
and `jira` modules serve the stored legacy ids again, since the migration
never changes `deckpilot.db`. Going back with plugins means restoring the
`plugins-schema5-…` backup as `plugins.db`.

Legacy action ids of the removed built-ins (WP-C5, WP-C6,
[marketplace/module-migration.md](marketplace/module-migration.md) §11 D1,
D2): this build has no `github`, `gitlab`, `jira`, `git`, `docker`,
`workspace` or `projects` module; their bundled first-party plugins
(`deckpilot.<id>`) are the only implementation. Every legacy id
(`github.pull-requests`, `projects.cycle`) stays in `ACTION_ALIASES`
(`crates/deckpilot-core/src/alias.rs`) for good, and stored profiles,
layouts, workflows and exports are never rewritten. While the plugin does
not serve (not installed, disabled or uninstalled by the user, waiting for
consent, plugins off with `--disable-plugins`), a legacy id is unavailable
with the reason ("`<legacy>` now comes from <Plugin> (`<id>`): …"), never
an unknown type; there is no built-in fallback. Once the plugin serves, the
same keys and nodes run again unchanged. An upgrade from 0.9.x migrates
without the built-in crates: static descriptors
(`apps/agent/src/plugins/migration/descriptors.rs`, X13) read the stored
`modules` rows, and a removed module's saved settings go to
`ModuleRegistry::set_legacy_config`, so a per-action `credential` override
naming the module's own credential is still dropped. Settings, credentials,
projects and resource bindings are kept; without the Projects plugin
project switching is refused ("the Projects plugin is not active"), and
nothing is deleted. Tests: `apps/agent/tests/core_without_plugins.rs`
(WP-T1), the `plugins_cutover*.rs` upgrades, and
`crates/deckpilot-modules/tests/contracts.rs` against the frozen 0.9.0
contracts in `crates/deckpilot-modules/legacy/`.

Before it, protocol 13 and plugin API 1.6: localization
([i18n.md](i18n.md#plugin-localization),
[marketplace/sdk/localization.md](marketplace/sdk/localization.md)).
Protocol 13 adds the `locale` capability: the `setLocale` request, with
which the app tells the agent its effective language (a tag such as `pl`),
and the optional `locale` field of the status. The client never sends
`setLocale` to an agent below protocol 13 (`AgentClient::set_locale`
returns without sending, a raw request is refused locally with
`Unsupported`), and the agent refuses it from a connection of an older
protocol; such an agent, and every agent before an app said anything,
draws its texts in English and shows plugin texts in the plugin's
fallback chain from English. After a change the agent announces the
plugins whose manifests name translations with the existing `changed`
plugin events (protocol 10), which older apps already reload on. The
plugin runtime lists the feature `locale` in `getPluginRuntime`. Plugin API
1.6 adds the manifest's `locales`, `defaultLocale` and `t:` texts and the
capability operations `locale.get` and `locale.translate` (compatible
additions to ABI 1): a manifest with `locales` or `defaultLocale` needs
`^1.6` (the validator refuses it below), so an agent of plugin API 1.5
reports such a version `incompatible`; below 1.6 a `t:` text is plain text,
so older manifests are unchanged. An agent before 1.6 answers the new
operations `Unsupported`.

Before it, protocol 12 and plugin API 1.5: plugin
dialogs (`dialog.open`, the `dialogs` permission, [dialogs.md](dialogs.md)).
Protocol 12 adds the `pluginDialogs` capability: `dialogOpened` and
`dialogClosed` plugin events, sent only to a connection of protocol 12 that
announced the feature (`subscribePluginEvents { features: ["pluginDialogs"] }`),
and the `answerPluginDialog` request. A protocol 11 app never announces it,
so it receives none of them, and an agent of protocol 11 ignores the
announcement; nothing an 0.9.0 app or agent receives changes. A plugin that
asks for a dialog while no announcing app is connected gets `unavailable`;
a manifest with the `dialogs` permission needs `^1.5`, so a 0.9.0 agent
(1.4) reports it `incompatible`.

Besides these, 0.9.0 provides plugin API 1.4 (0.8.0 and 0.8.1 provide
1.3): a view's `context` and a manifest's `dependencies` and
`requiresDependencies` need it, and older agents refuse a manifest that
uses them; manifests without them install as before. The plugin store
(`plugins.db`, separate from the configuration database) goes from
schema 4 to 5, an additive table for dependencies, after an encrypted
backup in `plugins/.backups/`; a 0.8.1 agent refuses the upgraded file
without writing to it.

Schema 10 moves layouts, action instances and icons into shared tables
and rewrites the profile documents (document version 6); see
[Shared layouts](data-model.md#shared-layouts). Shared layouts need no
protocol of their own (protocol 11 is the live key states of the device
editor): the agent reports the layout each device shows in the existing
`DeviceInfo.layout` and the optional `DeviceInfo.previousLayout`
(older clients ignore it), and *Switch layout* and *Back to previous layout* are
action types of the `deck` module, which need no new request. An app or
agent of 0.6.0–0.8.1 refuses a schema 10 database before writing anything
(“schema version 10 is newer than supported version 9”), as earlier
releases refused newer schemas; going back means restoring the
`deckpilot-schema9-…` backup. A 0.8.1 agent that still had the database
open when a newer app upgraded it would read profile documents of version
6, which it refuses, instead of seeing profiles without layouts. Exports
without layouts of no profile stay payload version 2, which 0.6.0–0.8.1
import; an export of everything with such layouts is version 3, which
they refuse.

Schema 11 gives folder pages their protected Back key and Auto Exit
([The protected Back key](data-model.md#the-protected-back-key)): layout
documents gain the optional page fields `back` and `autoExit` (item
document version 1 is unchanged; the fields are additive) and migration 11
rewrites stored layouts after the usual encrypted backup
(`deckpilot-schema10-…`). A 0.9.0 app or agent refuses a schema 11 database
before writing anything (“schema version 11 is newer than supported
version 10”); going back means restoring that backup. No IPC change: the
reserved Back and Auto Exit are the agent's own, the move is reported in
the existing device status, and preview faces mark the key with an optional
`navigation.system` field, which older callers ignore. Exports carry the
fields; a 0.9.0 import ignores them (its folders get the automatic Back on
the first free key, as before), and importing an older export here applies
the migration rule. Plugin API stays 1.7.

Schema 12 makes folder keys core navigation controls
([Folder controls](data-model.md#folder-controls)): layout documents gain
the optional `recovered` list (item document version 1 is unchanged; the
field is additive) and migration 12 rewrites stored layouts after the usual
encrypted backup (`deckpilot-schema11-…`). An app or agent of schema 11
refuses a schema 12 database before writing anything (“schema version 12
is newer than supported version 11”); going back means restoring that
backup. No IPC change (protocol and plugin API stay as they are): folder
keys are decided from the bindings the agent already reads, and
`save_layout` refusals use new `invalid.folderControl.*` codes that older
apps show by their English message. Exports carry `recovered`; an older
import ignores it (the recovered bindings are lost there, the folder keys
work). Importing an older export here normalizes its folder keys with a
warning.

Schema 13 makes a folder's name the one title of its key
([key-text.md](key-text.md#folder-names)): migration 13 (after the usual
encrypted backup, `deckpilot-schema12-…`) drops a folder key's own title
equal to the folder's name and moves one that differs to the layout's
`recovered` list with the reason `folderLabel` and a migration note; shared
layouts are rewritten once, layouts without such titles stay byte for byte
the same. An app or agent of schema 12 refuses a schema 13 database before
writing anything; going back means restoring that backup. Imports and
saving apply the same rule (an import says so in a warning). No IPC change.
An older import of a new export reads `folderLabel` as an unknown reason
and refuses the layout, as with any newer value.

Text appearances of key titles (wrap, single line, auto fit, marquee;
[key-text.md](key-text.md)) are an optional `text` field of a binding's
appearance: no schema, document, payload or protocol change. A 0.9.0 agent
ignores the field and draws the title wrapped; a 0.9.0 app drops it when it
saves that layout. The app's layout preview reports how titles come out in
an optional `text` field of each control's face, which older callers
ignore.

DeckPilot 0.5.0 was never published. Its two draft prereleases (0.5.0-1
and 0.5.0-2) have since been deleted; only the tag `v0.5.0-2` remains, as
the compatibility baseline that `tools/compat/matrix.sh` builds. "0.5.0" in
this page means that build.

## Protocol negotiation

- The app connects with the newest protocol it speaks. An agent before
  protocol 6 refuses a newer `hello` (`unsupportedProtocol`), so the client
  steps down one version at a time until one is accepted, down to protocol
  4 (`MIN_COMPATIBLE_AGENT_PROTOCOL`). The connection then uses the lower of
  the two protocols.
- What a connection can do is a list of **capabilities** derived from that
  protocol (`Capability::of_protocol`): `workflows` and `prepareForUpdate`
  (2), `moduleNamespaces` (3), `encryptedDatabase` (4),
  `backgroundExecutions` (5), `notifications` (6), `variables` (7),
  `workSession` (8), `variableRegistry` (9), `plugins` (10), `keyStates`
  (11), `pluginDialogs` (12), `locale` (13), `simulateInput` (14),
  `restoreBundled` (15), `livePreview` (16), `displayTemplates` and
  `keyLabels` (17). Agents
  from protocol 6
  also list them in `welcome`; the client never credits an agent with more
  than its protocol has.
- **No unsupported requests.** Every request has the protocol that
  introduced it (`Request::since_protocol`). The client refuses a newer
  request before sending it (`ClientError::Unsupported`, "the running agent
  (0.5.0-2) is too old for `listExecutions` … restart the agent"), and the
  agent refuses one sent anyway (`unsupported`). Events newer than a
  client's protocol are not sent to it.
- Agents before protocol 4 (DeckPilot 0.3 and earlier) cannot be used at
  all: the app reports **update required** and can only stop them (with
  protocol 1) and start its own agent.

## Compatibility states in the app

| State | When | What the app does |
| ----- | ---- | ----------------- |
| compatible | same version | everything available |
| agent outdated | agent older (e.g. after an update) | works with the negotiated protocol; names the features that are unavailable until the agent is restarted; offers the restart |
| agent newer | agent newer than the app | same, from the app's side; the app's own (older) agent must not be started against the newer database (it refuses it, see below) |
| update required | agent below protocol 4 | nothing configurable; offers to replace the agent |
| unknown | a version cannot be parsed | offers the restart |
| other distribution | the agent was delivered by another distribution channel than the app, for example an APT agent next to a GitHub app (rule R6 of [distribution/README.md](distribution/README.md#5-cross-channel-rejection-rules)) | says which installation each belongs to; offers the restart, which replaces it with the agent of the app's own bundle. Checked before the version |
| stopped | no agent | offers to start it; reconnects automatically when it runs again |

The channel comes from the agent's distribution record, an optional
`distribution` field in its status (`getStatus`, protocol 10 unchanged):
the channel, update provider, package format where the build knows it,
release channel, version, platform and architecture, all compiled into
the agent. Older apps ignore the field; agents of 0.6.0-alpha.3 and
earlier do not send it and were all GitHub builds, so the app treats them
as such and checks only their version, as before. A channel name the app
does not know counts as another channel.

The app keeps a subscription to the agent and reconnects every 2 s after
the agent stops or restarts; each reconnection negotiates again, so an
agent that was replaced by another version while the app stayed open is
recognised with its new version, protocol and capabilities.

## Matrix

"Real" means the actual binaries of both versions; "simulated" means a test
peer that behaves like the other version, using messages recorded from the
real binary (`crates/deckpilot-ipc/tests/fixtures/`). Results marked
*measured before 0.6.0-alpha.2* were recorded when main spoke protocol 6
with schema 7 and have not been recorded again for protocol 9 and schema 9.

| Case | How it is tested | Result |
| ---- | ---------------- | ------ |
| 0.5.0 app/client with 0.5.0 agent | real (`tools/compat/matrix.sh`) | works (protocol 4) |
| 0.6.0 client with 0.6.0 agent | real (matrix), agent and desktop tests | works (protocol 6; measured before 0.6.0-alpha.2) |
| 0.6.0 client with 0.5.0 agent | real (matrix: `real_agent` test against the running 0.5.0-2 agent); simulated (`peers` tests) | negotiates protocol 4 after refused `hello` 6 and 5; status read; `listExecutions` refused locally and never sent (measured before 0.6.0-alpha.2) |
| 0.5.0 client with 0.6.0 agent | real (matrix: the 0.5.0-2 CLI client); agent test with recorded protocol 4 messages | works with protocol 4; `welcome` lists protocol 4's capabilities; protocol 5 requests refused with `unsupported` |
| 0.8.1 agent given a schema 10 database | real (`tools/compat/matrix.sh v0.8.1`) | refused before anything is written; the database and key file stay byte-identical; this build still opens it |
| this build given a 0.8.1 database | real (matrix: database created by the 0.8.1 tools); storage tests on the `v0.8.1` fixture | upgraded to schema 10; profiles, layouts in order, bindings, active layouts, actions, icons, active profile and project kept |
| 0.5.0 agent given a 0.6.0 database | real (matrix) | refused ("schema version 7 is newer than supported version 5"); database and key file byte-identical afterwards; the 0.6.0 agent still opens it (measured before 0.6.0-alpha.2) |
| 0.6.0 agent given a 0.5.0 database | real (matrix: database created by the 0.5.0-2 tools) | upgraded to schema 7; profiles, active profile and project kept (measured before 0.6.0-alpha.2) |
| agent before protocol 4 | simulated (`peers`: a protocol 3 peer) | update required; stopping it with protocol 1 still works |
| protocol 5 agent | simulated (`peers`) | background executions without notifications |
| protocol 9 agent (before plugins) | simulated (`peers`) | every plugin request refused locally (`Unsupported`, needs protocol 10) and never sent |
| protocol 10 agent without plugins (any release build) | agent tests (`plugins_e2e`, `with_plugins_disabled_nothing_is_available`) | `getPluginRuntime` says why; every other plugin request fails with `unsupported` and reason `pluginsUnavailable`; plugin events are a separate message sent only after `subscribePluginEvents`, so older clients never see them |
| protocol 11 app or agent with a protocol 12 peer (plugin dialogs) | agent test `plugin_dialogs_reach_only_connections_that_announced_them`; IPC test `plugin_dialogs_are_protocol_12_and_announced` | the client leaves the announcement out below protocol 12 and the agent ignores it there; `answerPluginDialog` refused (`unsupported`) locally and by the agent; no dialog event reaches a protocol 11 connection or one that did not announce; with no announcing app a plugin's dialog is `unavailable` |
| protocol 12 app or agent with a protocol 13 peer (the app's language) | IPC test `a_protocol_12_agent_is_never_told_the_locale`; agent test `a_protocol_12_app_never_sets_the_language` (`tests/locale.rs`); IPC test `the_locale_is_protocol_13_and_only_a_tag` | the client does not send `setLocale` below protocol 13 and refuses it locally; the agent refuses it from an older connection; the status of an older agent has no `locale`; keys and plugin texts stay English; everything else works as before |
| protocol 13 app or agent with a protocol 14 peer (simulated presses) | IPC test `a_protocol_13_agent_is_never_asked_to_simulate_a_press`; agent test `a_protocol_13_app_cannot_simulate` (`tests/simulate_press.rs`); IPC test `simulated_presses_are_protocol_14`; desktop test `simulated_presses_answer_ran_confirmation_or_refused` | the client does not send `simulatePress` below protocol 14 and refuses it locally (the app answers `refused` with code `unsupported`); the agent refuses it from an older connection; nothing is pressed; everything else works as before |
| protocol 15 app or agent with a protocol 16 peer (live preview) | IPC test `a_protocol_15_agent_is_never_asked_for_a_live_preview`; agent test `a_protocol_15_connection_cannot_preview` (`tests/live_preview.rs`); IPC test `live_preview_is_protocol_16`; frontend `use-live-preview.test.tsx` | the client does not send the live preview requests below protocol 16 and refuses them locally; the agent refuses them from an older connection; the editor says changes reach the device when saved; everything else works as before |
| protocol 17 app or agent with a protocol 18 peer (structured variables) | IPC tests `structured_variables_need_protocol_18` (`protocol.rs`) and `a_protocol_17_agent_has_no_structured_variables` (`tests/peers.rs`) | an older app receives structured types as `json` without members or keys, no members-only entries, structured previews as `redacted` and the nearest older missing reasons; nothing it sends changes; a newer app with an older agent sees no repository variables |
| protocol 18 app or agent with a protocol 19 peer (Calls) | IPC test `a_protocol_18_agent_is_never_asked_about_calls` (`tests/peers.rs`); agent test `a_protocol_18_connection_cannot_ask_about_calls` (`tests/calls.rs`); IPC test `calls_need_protocol_19_and_carry_no_private_text` | the client does not send `getCallsStatus`, `setCallsOverlay` or `restoreCallsLayout` below protocol 19 and refuses them locally; the agent refuses them from an older connection and sends it no `callsChanged`; the app's Calls settings stay hidden; everything else works as before |
| protocol 18 app or agent with a protocol 20 peer (plugin connections) | IPC tests `a_protocol_18_agent_is_never_sent_a_connection_or_its_token` (`tests/peers.rs`) and `requests_are_checked_and_never_show_the_token` (`connections.rs`) | the client does not send the connection requests below protocol 20 and refuses them locally, so no token leaves the app; the agent refuses them from an older connection; an older app shows no Connections settings; everything else works as before |
| protocol 16 app or agent with a protocol 17 peer (variables in key titles, key labels) | IPC test `a_protocol_16_agent_is_never_asked_to_resolve_display_templates` (`tests/peers.rs`) | the client does not send `listDisplayVariables` or `resolveDisplayTemplates` below protocol 17 and refuses them locally; the app shows titles resolved by the core alone; labels travel only as optional fields that an older peer ignores; everything else works as before |
| agent temporarily unavailable | desktop tests (`stopped` state), connect timeouts (`deckpilot-ipc`) | stopped state; no request hangs |
| agent restarted during a running workflow | agent test `runs_of_a_stopped_agent_are_recorded_as_interrupted` | run recorded as interrupted; history kept |
| agent version changed while the app stays open | bridge reconnect (negotiates again); simulated peers per version | covered by the same negotiation; not tested end to end with a running app window |

Run the real matrix locally with `tools/compat/matrix.sh [<old tag>] [<work dir>]`;
CI runs it on Linux for changes to the IPC, storage or agent code
(`.github/workflows/compatibility.yml`). It uses throwaway data protected by
the development password, never a credential store.

## Lifecycle

| Situation | Behaviour | Evidence |
| --------- | --------- | -------- |
| Fresh installation | database created by the agent with a system key; empty Action Library | agent/desktop tests (`a_fresh_install_…`, `fresh_installation_has_an_empty_action_library`) |
| Agent already running | a second agent exits (instance lock) | `single_instance_shutdown_and_client_reconnect` |
| App closed while the agent runs | the agent and its runs continue | agent runs are independent of clients (`device_bound_workflows_run_in_the_background`) |
| App reopened | reuses the unlocked agent | `a_restarted_app_reuses_the_unlocked_agent_without_asking` |
| Agent crash or stop during a run | runs marked interrupted on stop and on the next start | `runs_of_a_stopped_agent_are_recorded_as_interrupted` |
| Agent restart from the app | refused while workflows run; with confirmation the runs are interrupted and recorded | `restarting_the_agent_never_ends_running_workflows_silently` |
| App/agent update | installation waits while workflows run (automatic installation retries later; manual installation says why); the intent is recorded, the old agent's endpoint and instance lock are both gone before anything is installed, and the next start completes or reports the update ([updates.md](updates.md#recovery-after-an-interrupted-update)) | `InstallContext::agent_busy` from `runningExecutions`; policy tests; `updates_tests.rs` (`install`, `recovery`) |
| Agent of another installation | refused (other distribution); the restart starts this app's agent only once the other one released the instance lock | `deckpilot-update` compatibility tests; `agent.rs` tests |
| IPC endpoint conflicts | endpoint per data directory; bounded connect | `deckpilot-ipc` endpoint tests |
| Incompatible pair | old agent refuses the newer database without changing it | matrix |
| System restart, logout/login | agent started by autostart; database opens with the system key | manual (release validation); not automated |
| Multiple app windows | one app process; module windows by identifier | `native_windows_are_opened_by_identifier_and_reused` |

An update never ends a running workflow silently: it waits, or the user
cancels the runs, which the history records as cancelled or interrupted.

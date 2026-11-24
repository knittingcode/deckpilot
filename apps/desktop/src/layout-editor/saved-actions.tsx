import { type ReactNode, useCallback, useState } from "react"

import { ActionEditor } from "@/components/action-editor"
import { ReferenceList } from "@/components/action-references"
import { ModalShell } from "@/components/modal-shell"
import { useTranslation } from "@/i18n"
import { actionReferences, isDirect, referenceTarget } from "@/lib/action-references"
import { useActionModules, useAppContext } from "@/lib/app-context"
import { withLegacyAliasModules } from "@/lib/plugin-actions"
import { requestFocus } from "@/lib/focus"
import type { ActionInstance, ActionReference, ProfileView } from "@/lib/types"

import { ActionSetup } from "./action-setup"
import { SharedAction } from "./shared-action"

/**
 * The saved action a key or gesture runs, in the Inspector: how many
 * places share it, what it still needs, and the only ways left to manage
 * it now that the library lists types alone (no "My actions", no Action
 * Library page): its definition, in the Action Library's editor, which
 * says it changes every place that uses it, and where it is used. A
 * binding's own overrides stay in its card ("this key/gesture only").
 * Saved actions used only by workflows are edited in the workflow editor.
 */

type Open = { kind: "edit"; action: ActionInstance } | { kind: "usages"; action: ActionInstance }

export function useSavedActionTools({ profileId, view }: { profileId: string; view: ProfileView }) {
  // The draft whose first missing field takes focus once it is shown.
  const [focusSetup, setFocusSetup] = useState<string | null>(null)
  const focused = useCallback(() => {
    setFocusSetup(null)
  }, [])
  const { t } = useTranslation()
  const { profiles, agent, refresh, navigate } = useAppContext()
  // Legacy ids a plugin replaces are edited with the plugin's fields.
  const modules = withLegacyAliasModules(useActionModules())
  const [open, setOpen] = useState<Open | null>(null)
  const close = () => {
    setOpen(null)
  }
  const profile = view.profile
  const actionOf = (id: string) => profile.actions.find((a) => a.id === id)
  const referencesOf = (action: ActionInstance) => actionReferences(profile, action.id)
  const usedBy = (action: ActionInstance) => referencesOf(action).filter(isDirect).length
  const typeOf = (action: ActionInstance) =>
    modules.flatMap((m) => m.actionTypes).find((type) => type.id === action.actionType)
  const canRun = Boolean(agent?.running) && profiles.activeProfileId === profileId
  const openReference = (reference: ActionReference) => {
    const target = referenceTarget(reference)
    close()
    requestFocus(target.focus)
    navigate(target.page)
  }

  /** The Inspector's line for the saved action `id` a binding runs. */
  const settingsFor = (id: string): ReactNode => {
    const action = actionOf(id)
    if (!action) return null
    const type = typeOf(action)
    const users = usedBy(action)
    return (
      <SharedAction
        key={id}
        action={action}
        type={type}
        users={users}
        setup={
          // Its own draft: its required settings right here. Shared ones
          // are filled in through their definition, for every place.
          type && users <= 1 ? (
            <ActionSetup
              key={id}
              profileId={profileId}
              action={action}
              type={type}
              projects={profile.projects}
              autoFocus={focusSetup === id}
              onFocused={focused}
              onSaved={refresh}
            />
          ) : undefined
        }
        onEdit={() => {
          setOpen({ kind: "edit", action })
        }}
        onUsages={() => {
          setOpen({ kind: "usages", action })
        }}
      />
    )
  }

  const closeButton = { label: t("common:close"), onClick: close, variant: "outline" as const }
  const dialogs =
    open === null ? null : open.kind === "usages" ? (
      <ModalShell
        presentation="dialog"
        open
        size="medium"
        title={t("editor:savedAction.usagesTitle", { name: open.action.name })}
        onClose={close}
        secondary={closeButton}
      >
        <ReferenceList references={referencesOf(open.action)} onOpen={openReference} />
      </ModalShell>
    ) : (
      <ModalShell
        presentation="dialog"
        open
        size="large"
        title={open.action.name}
        description={
          usedBy(open.action) > 1
            ? t("editor:savedAction.sharedEdit", { count: usedBy(open.action) })
            : undefined
        }
        onClose={close}
        secondary={closeButton}
      >
        <ActionEditor
          key={open.action.id}
          profileId={profileId}
          modules={modules}
          projects={profile.projects}
          workflows={profile.workflows}
          action={actionOf(open.action.id) ?? open.action}
          references={referencesOf(open.action)}
          candidates={profile.actions}
          canRun={canRun}
          onSaved={() => {
            refresh()
          }}
          onDeleted={() => {
            close()
            refresh()
          }}
          onOpenReference={openReference}
        />
      </ModalShell>
    )

  return {
    settingsFor,
    dialogs,
    /** After a drop of a draft: its first missing field takes focus when shown. */
    focusSetupOf: setFocusSetup,
  }
}

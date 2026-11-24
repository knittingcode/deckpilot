import { useState } from "react"

import { ConfirmDialog } from "@/components/dialogs"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"

/**
 * What a saved use of an action can become when the action does not work
 * where it is (design §6 of the action-context audit). Each change asks
 * for confirmation; "Keep" leaves everything as it is (the saved
 * configuration is never changed by itself).
 */
export interface MigrationOffer {
  /** The action's name, for the dialogs. */
  name: string
  /** Wrap it in a new Multi Action that this control runs. */
  wrap?: (() => Promise<void> | void) | undefined
  /** Move it into a new workflow that this control starts. */
  move?: (() => Promise<void> | void) | undefined
  /** Remove the step (a Multi Action's step that cannot run there). */
  remove?: (() => void) | undefined
}

type Pending = "wrap" | "move" | "remove" | null

export function ContextMigrations({ offer }: { offer: MigrationOffer | undefined }) {
  const { t } = useTranslation()
  const [kept, setKept] = useState(false)
  const [pending, setPending] = useState<Pending>(null)
  if (!offer || kept || (!offer.wrap && !offer.move && !offer.remove)) return null
  const name = offer.name
  const dialog = {
    wrap: {
      title: t("editor:migrate.wrapTitle", { name }),
      description: t("editor:migrate.wrapText", { name }),
      confirm: t("editor:migrate.wrapSequence"),
      run: offer.wrap,
    },
    move: {
      title: t("editor:migrate.moveTitle", { name }),
      description: t("editor:migrate.moveText", { name }),
      confirm: t("editor:migrate.moveWorkflow"),
      run: offer.move,
    },
    remove: {
      title: t("editor:migrate.removeTitle", { name }),
      description: t("editor:migrate.removeText"),
      confirm: t("editor:migrate.removeStep"),
      run: offer.remove,
    },
  }
  const open = pending ? dialog[pending] : null
  return (
    <div
      className="flex flex-wrap gap-1.5"
      role="group"
      aria-label={t("editor:context.notHere")}
      data-slot="context-migrations"
    >
      {offer.wrap && (
        <Button
          size="xs"
          variant="outline"
          onClick={() => {
            setPending("wrap")
          }}
        >
          {t("editor:migrate.wrapSequence")}
        </Button>
      )}
      {offer.move && (
        <Button
          size="xs"
          variant="outline"
          onClick={() => {
            setPending("move")
          }}
        >
          {t("editor:migrate.moveWorkflow")}
        </Button>
      )}
      {offer.remove && (
        <Button
          size="xs"
          variant="outline"
          onClick={() => {
            setPending("remove")
          }}
        >
          {t("editor:migrate.removeStep")}
        </Button>
      )}
      <Button
        size="xs"
        variant="ghost"
        onClick={() => {
          setKept(true)
        }}
      >
        {t("editor:migrate.keep")}
      </Button>
      <ConfirmDialog
        open={open !== null}
        onOpenChange={(next) => {
          if (!next) setPending(null)
        }}
        destructive={pending === "remove"}
        title={open?.title ?? ""}
        description={open?.description ?? ""}
        confirmLabel={open?.confirm ?? ""}
        onConfirm={() => {
          const run = open?.run
          setPending(null)
          if (run) void run()
        }}
      />
    </div>
  )
}

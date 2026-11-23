import { Check, ChevronDown, LayoutGrid, Library, Monitor, Plus, Star } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { FormDialog } from "@/components/dialogs"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useAsync } from "@/hooks/use-async"
import { useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { failureMessage } from "@/lib/failure"
import { shownLayout } from "@/lib/layout-navigation"
import type { DeviceModel } from "@/lib/types"
import { cn } from "@/lib/utils"
import { selectedDevice } from "@/shell/device-options"
import { useEditorSelection } from "@/shell/editor-selection-context"

import { defaultLayoutOf, disambiguate, resolveLayout, useConfirmDiscard } from "./layout-selection"
import { useSetDefaultLayout } from "./set-default-layout"

/**
 * Layout switching of the Device Editor, shown in the main window's
 * toolbar (`LayoutSelector` in shell/toolbar.tsx): only the layouts of the
 * profile being edited that fit the device's model, a new layout (created
 * in that profile), and the way to the Layout Manager (pages/layouts.tsx),
 * which duplicates, renames, sets defaults, deletes, assigns layouts to
 * profiles and manages other models' layouts. Nothing here copies, moves,
 * assigns or overwrites a layout, and no other profile is written.
 */

type Dialog = { kind: "new" }

/**
 * The toolbar's layout selector: a switcher between the layouts that fit
 * the device's model. Layouts of other models are managed in the Layout
 * Manager.
 */
export function LayoutMenu({ className }: { className?: string }) {
  const { t } = useTranslation()
  const { editingProfileId, revision, refresh, agent, navigate } = useAppContext()
  const selection = useEditorSelection()
  const confirmDiscard = useConfirmDiscard()
  const profileId = editingProfileId
  const view = useAsync(
    () => (profileId ? api.getProfile(profileId) : Promise.resolve(null)),
    `layout-menu:${profileId ?? ""}:${revision}`,
  )
  const sharedState = useAsync(() => api.listSharedLayouts(), `layout-menu-shared:${revision}`)
  const [dialog, setDialog] = useState<Dialog | null>(null)

  const models = selection.models.status === "ready" ? selection.models.data : []
  const model = models.find((m) => m.id === selection.modelId)
  const profile = view.state.status === "ready" ? view.state.data?.profile : undefined
  const all = profile?.layouts ?? []
  const layouts = model ? all.filter((l) => l.model === model.id) : []
  const device = model
    ? selectedDevice(agent?.status?.devices ?? [], model.id, selection.device)
    : null
  const onDevice = shownLayout(device)
  const defaultLayout = model
    ? defaultLayoutOf(
        layouts,
        profile?.activeLayouts?.[model.id],
        onDevice,
        agent?.status?.activeProfile?.id === profileId,
      )
    : null
  const current = resolveLayout(layouts, selection.layoutId, defaultLayout)
  // The profile's default for the model, as stored (the star; the device
  // may show another layout, which the editor follows when opening).
  const stored = profile?.activeLayouts?.[model?.id ?? ""]
  const profileDefault = layouts.some((l) => l.id === stored) ? stored : layouts[0]?.id
  const { pending, setDefault } = useSetDefaultLayout(refresh)
  const shared = sharedState.state.status === "ready" ? sharedState.state.data : null
  const summary = (id: string) => shared?.find((l) => l.id === id)
  const contents = (id: string) => {
    const s = summary(id)
    return s
      ? t("layouts:page.contents", {
          pages: t("layouts:page.pages", { count: s.pages }),
          keys: t("layouts:page.keys", { count: s.bindings }),
        })
      : undefined
  }
  const notes = disambiguate(layouts, contents)
  const ready = Boolean(profileId && model && profile)

  const changed = (message: string, id?: string) => {
    toast.success(message)
    if (id && model) selection.showLayout(id, model.id)
    refresh()
  }
  const choose = async (id: string) => {
    if (id === current?.id) return
    if (!(await confirmDiscard())) return
    selection.selectLayout(id)
  }
  const label = current
    ? current.id === profileDefault
      ? t("layouts:menu.selectorDefault", { layout: current.name })
      : t("layouts:menu.selector", { layout: current.name })
    : t("layouts:menu.choose")
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className={className}
            aria-label={label}
            title={label}
            disabled={!ready}
          >
            <LayoutGrid />
            <span className="truncate">{current?.name ?? t("layouts:menu.placeholder")}</span>
            <ChevronDown className="opacity-60" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-72">
          <DropdownMenuGroup aria-label={t("layouts:menu.profileLayouts")}>
            {layouts.length === 0 && (
              <DropdownMenuItem disabled>{t("layouts:menu.empty")}</DropdownMenuItem>
            )}
            {layouts.map((layout) => {
              const note = notes.get(layout.id)
              const isDefault = layout.id === profileDefault
              return (
                // A row of two menu items: the layout, and its star after it
                // (arrow keys reach both, in this order).
                <div key={layout.id} className="flex items-center gap-0.5">
                  <DropdownMenuItem
                    className="min-w-0 flex-1"
                    aria-current={layout.id === current?.id ? "true" : undefined}
                    title={layout.name}
                    onSelect={() => {
                      void choose(layout.id)
                    }}
                  >
                    <Check className={cn(layout.id !== current?.id && "invisible")} aria-hidden />
                    <span className="grid min-w-0 flex-1">
                      <span className="truncate">{layout.name}</span>
                      {note && (
                        <span className="truncate text-xs text-muted-foreground">{note}</span>
                      )}
                    </span>
                    {layout.id === onDevice && (
                      <Monitor
                        className="text-success"
                        aria-label={t("layouts:menu.onDevice")}
                        role="img"
                      />
                    )}
                  </DropdownMenuItem>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <DropdownMenuItem
                        data-slot="default-star"
                        data-state={isDefault ? "on" : "off"}
                        role="menuitemcheckbox"
                        aria-checked={isDefault}
                        aria-label={
                          isDefault
                            ? t("layouts:menu.star.isDefaultLabel", { layout: layout.name })
                            : t("layouts:menu.star.setDefaultLabel", { layout: layout.name })
                        }
                        disabled={pending !== null}
                        className={cn(
                          "size-7 shrink-0 justify-center p-0 focus-visible:ring-[3px] focus-visible:ring-ring/50",
                          isDefault ? "cursor-default" : "focus:[&_svg]:text-warning",
                        )}
                        onSelect={(e) => {
                          // Stays open, on the layout chosen before.
                          e.preventDefault()
                          if (!isDefault && profileId && model) {
                            void setDefault(profileId, layout, model.name)
                          }
                        }}
                      >
                        <Star
                          aria-hidden
                          className={cn(
                            isDefault ? "fill-warning text-warning" : "text-muted-foreground",
                          )}
                        />
                      </DropdownMenuItem>
                    </TooltipTrigger>
                    <TooltipContent>
                      {isDefault ? t("layouts:menu.star.default") : t("layouts:menu.star.set")}
                    </TooltipContent>
                  </Tooltip>
                </div>
              )
            })}
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup aria-label={t("layouts:menu.manageGroup")}>
            <DropdownMenuItem
              disabled={!ready}
              onSelect={() => {
                setDialog({ kind: "new" })
              }}
            >
              <Plus /> {t("layouts:menu.new")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onSelect={() => {
                navigate("layouts")
              }}
            >
              <Library /> {t("layouts:menu.manage")}
            </DropdownMenuItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      {profileId && model && dialog?.kind === "new" && (
        <NewLayoutDialog
          model={model}
          onClose={() => {
            setDialog(null)
          }}
          onCreate={async (name, generate) => {
            if (!(await confirmDiscard())) return
            const { id } = await api.createLayout(profileId, model.id, name, generate)
            setDialog(null)
            changed(t("layouts:page.toast.created"), id)
          }}
        />
      )}
    </>
  )
}

/** A new layout of the profile for one device model. */
function NewLayoutDialog({
  model,
  onClose,
  onCreate,
}: {
  model: DeviceModel
  onClose: () => void
  /** Rejects with the backend's error; the dialog then stays open and shows it. */
  onCreate: (name: string, generate: boolean) => Promise<void>
}) {
  const { t } = useTranslation()
  const [name, setName] = useState("")
  const [generate, setGenerate] = useState(true)
  return (
    <FormDialog
      open
      title={t("layouts:management.newTitle", { model: model.name })}
      submitLabel={t("layouts:page.create.submit")}
      dirty={name.trim() !== ""}
      onClose={onClose}
      onSubmit={() => onCreate(name, generate)}
      describeError={(e) =>
        failureMessage(t("layouts:management.createFailed", { layout: name }), e)
      }
    >
      <div className="grid gap-2">
        <Label htmlFor="layout-name">{t("layouts:page.create.name")}</Label>
        <Input
          id="layout-name"
          value={name}
          autoFocus
          placeholder={t("layouts:management.newPlaceholder", { model: model.name })}
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
      <label className="flex items-center gap-3 text-sm">
        <Switch checked={generate} onCheckedChange={setGenerate} />
        {t("layouts:management.generate")}
      </label>
    </FormDialog>
  )
}

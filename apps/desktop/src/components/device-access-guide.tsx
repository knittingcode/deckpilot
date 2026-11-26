import { ClipboardCopy, Usb } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ModalShell } from "@/components/dialogs"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"
import {
  type DeviceAccessIssue,
  INSTALLATION_URL,
  type RuleSource,
  TROUBLESHOOTING_URL,
  deviceAccessIssue,
  readDismissed,
  ruleSource,
  udevCommands,
  writeDismissed,
} from "@/lib/device-access"

/** Shows the guide once per issue while the app runs (mounted in the shell). */
export function DeviceAccessGuide() {
  const { agent, updates } = useAppContext()
  const [dismissed, setDismissed] = useState(readDismissed)
  const [shown, setShown] = useState<DeviceAccessIssue | null>(null)
  const issue = deviceAccessIssue(agent, updates)
  // Once shown, the guide stays until it is closed, even if the device
  // recovers meanwhile (state adjusted while rendering, not in an effect).
  if (shown === null && issue !== null && !dismissed.includes(issue)) {
    setShown(issue)
  }

  if (shown === null) return null
  const close = () => {
    const next = [...new Set([...dismissed, shown])]
    writeDismissed(next)
    setDismissed(next)
    setShown(null)
  }
  return (
    <DeviceAccessDialog
      issue={shown}
      source={ruleSource(updates?.packageFormat)}
      version={updates?.currentVersion ?? agent?.status?.version ?? "<version>"}
      target={updates?.target ?? ""}
      onClose={close}
    />
  )
}

function CopyBlock({ text, label }: { text: string; label: string }) {
  const { t } = useTranslation()
  return (
    <div className="grid gap-1">
      <pre
        className="relative overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs leading-5 whitespace-pre select-text"
        aria-label={label}
      >
        {text}
      </pre>
      <div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            navigator.clipboard.writeText(text).then(
              () => toast.success(t("devices:accessGuide.copied")),
              () => toast.error(t("devices:accessGuide.notCopied")),
            )
          }}
        >
          <ClipboardCopy /> {t("devices:accessGuide.copy", { what: label })}
        </Button>
      </div>
    </div>
  )
}

export function DeviceAccessDialog({
  issue,
  source,
  version,
  target,
  onClose,
}: {
  issue: DeviceAccessIssue
  source: RuleSource
  version: string
  target: string
  onClose: () => void
}) {
  const { t } = useTranslation()
  const permission = issue === "permission"
  return (
    <ModalShell
      presentation="dialog"
      open
      size="medium"
      icon={Usb}
      title={t(permission ? "devices:accessGuide.permissionTitle" : "devices:accessGuide.usbTitle")}
      description={t(
        permission ? "devices:accessGuide.permissionIntro" : "devices:accessGuide.usbIntro",
      )}
      onClose={onClose}
      primary={{ label: t("devices:accessGuide.done"), onClick: onClose }}
    >
      <div className="flex min-w-0 flex-col gap-3 text-sm">
        {permission ? (
          <>
            <p>{t(`devices:accessGuide.rule.${source}`)}</p>
            <CopyBlock
              text={udevCommands(source, version, target)}
              label={t("devices:accessGuide.commands")}
            />
            <p>{t("devices:accessGuide.replug")}</p>
            <p className="text-muted-foreground">{t("devices:accessGuide.neverRuns")}</p>
            <CopyBlock text={INSTALLATION_URL} label={t("devices:accessGuide.installationLink")} />
          </>
        ) : (
          <>
            <ul className="list-disc ps-5">
              <li>{t("devices:accessGuide.usbReplug")}</li>
              <li>{t("devices:accessGuide.usbDirect")}</li>
              <li>{t("devices:accessGuide.usbPowerCycle")}</li>
            </ul>
            <p className="text-muted-foreground">{t("devices:accessGuide.usbNoReset")}</p>
          </>
        )}
        <CopyBlock
          text={TROUBLESHOOTING_URL}
          label={t("devices:accessGuide.troubleshootingLink")}
        />
        <p className="text-muted-foreground">{t("devices:accessGuide.once")}</p>
      </div>
    </ModalShell>
  )
}

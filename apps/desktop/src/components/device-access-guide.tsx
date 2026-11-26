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

/** What the open guide shows: kept while it is open. */
interface Shown {
  issue: DeviceAccessIssue
  source: RuleSource
  version: string
  target: string
  channel: string
}

/** Shows the guide once per issue while the app runs (mounted in the shell). */
export function DeviceAccessGuide() {
  const { agent, updates } = useAppContext()
  const [dismissed, setDismissed] = useState(readDismissed)
  const [shown, setShown] = useState<Shown | null>(null)
  const issue = deviceAccessIssue(agent, updates, dismissed)
  // The permission fix depends on the package: wait for the update state
  // that says which one. Once shown, the guide and its text stay until it
  // is closed, even if the device recovers meanwhile (state adjusted while
  // rendering, not in an effect).
  if (shown === null && issue !== null && (issue !== "permission" || updates !== null)) {
    setShown({
      issue,
      source: ruleSource(updates?.packageFormat),
      version: updates?.currentVersion ?? agent?.status?.version ?? "<version>",
      target: updates?.target ?? "",
      channel: updates?.distributionChannel ?? "",
    })
  }

  if (shown === null) return null
  const close = () => {
    const next = [...new Set([...dismissed, shown.issue])]
    writeDismissed(next)
    setDismissed(next)
    setShown(null)
  }
  return <DeviceAccessDialog {...shown} onClose={close} />
}

function CopyBlock({ text, label, copy }: { text: string; label: string; copy: string }) {
  const { t } = useTranslation()
  return (
    <div className="grid gap-1" role="region" aria-label={label}>
      <pre className="relative overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs leading-5 whitespace-pre select-text">
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
          <ClipboardCopy /> {copy}
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
  channel,
  onClose,
}: Shown & { onClose: () => void }) {
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
              text={udevCommands(source, version, target, channel)}
              label={t("devices:accessGuide.commands")}
              copy={t("devices:accessGuide.copyCommands")}
            />
            <p>{t("devices:accessGuide.replug")}</p>
            <p className="text-muted-foreground">{t("devices:accessGuide.neverRuns")}</p>
            <CopyBlock
              text={INSTALLATION_URL}
              label={t("devices:accessGuide.installationLink")}
              copy={t("devices:accessGuide.copyLink")}
            />
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
          copy={t("devices:accessGuide.copyLink")}
        />
        <p className="text-muted-foreground">{t("devices:accessGuide.once")}</p>
      </div>
    </ModalShell>
  )
}

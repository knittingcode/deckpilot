import { save as saveDialog } from "@tauri-apps/plugin-dialog"
import { FileDown } from "lucide-react"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import { ModalShell } from "@/components/dialogs"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import type { DiagnosticsPreview } from "@/lib/types"

/**
 * "Export diagnostics…": prepares a support bundle (the diagnose report and
 * the end of the recent logs, redacted by the agent), shows all of it, and
 * saves it only to a file the user picks. Nothing is uploaded.
 */
export function ExportDiagnosticsButton() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button
        variant="outline"
        size="sm"
        onClick={() => {
          setOpen(true)
        }}
      >
        <FileDown /> {t("settings:diagnostics.export.button")}
      </Button>
      {open && (
        <ExportDiagnosticsDialog
          onClose={() => {
            setOpen(false)
          }}
        />
      )}
    </>
  )
}

export function ExportDiagnosticsDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation()
  const [preview, setPreview] = useState<DiagnosticsPreview | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    let current = true
    api.prepareDiagnostics().then(
      (prepared) => {
        if (current) setPreview(prepared)
      },
      (e: unknown) => {
        if (current) setError(failureMessage(t("settings:diagnostics.export.prepareFailed"), e))
      },
    )
    return () => {
      current = false
    }
  }, [t])

  const close = () => {
    if (saving) return
    void api.discardDiagnostics().catch(() => undefined)
    onClose()
  }

  const submit = async () => {
    if (!preview || saving) return
    setError(null)
    let path: string | null
    try {
      path = await saveDialog({
        defaultPath: preview.fileName,
        filters: [{ name: t("settings:diagnostics.export.fileType"), extensions: ["json"] }],
      })
    } catch (e) {
      setError(failureMessage(t("settings:diagnostics.export.saveFailed"), e))
      return
    }
    if (!path) return
    setSaving(true)
    try {
      await api.saveDiagnostics(preview.token, path)
      toast.success(t("settings:diagnostics.export.saved"))
      void api.discardDiagnostics().catch(() => undefined)
      onClose()
    } catch (e) {
      setError(failureMessage(t("settings:diagnostics.export.saveFailed"), e))
    } finally {
      setSaving(false)
    }
  }

  return (
    <ModalShell
      presentation="dialog"
      open
      size="large"
      icon={FileDown}
      title={t("settings:diagnostics.export.title")}
      description={t("settings:diagnostics.export.description")}
      error={error}
      loading={!preview && !error}
      onClose={close}
      onSubmit={() => void submit()}
      primary={{
        label: t("settings:diagnostics.export.save"),
        loadingLabel: t("settings:diagnostics.export.saving"),
        loading: saving,
        disabled: !preview,
      }}
      secondary={{ label: t("common:actions.cancel"), onClick: close, disabled: saving }}
    >
      {preview && (
        <div className="flex min-w-0 flex-col gap-3 text-sm">
          <div>
            <p className="font-medium">{t("settings:diagnostics.export.includes")}</p>
            <ul
              className="mt-1 list-disc ps-5 text-muted-foreground"
              aria-label={t("settings:diagnostics.export.includes")}
            >
              <li>{t("settings:diagnostics.export.report", { count: preview.findings })}</li>
              <li>{t("settings:diagnostics.export.app")}</li>
              {preview.logs.map((log) => (
                <li key={log.file}>
                  <span className="font-mono">{log.file}</span>
                  {": "}
                  {t(
                    log.truncated
                      ? "settings:diagnostics.export.logLinesEnd"
                      : "settings:diagnostics.export.logLines",
                    { count: log.lines },
                  )}
                </li>
              ))}
              {preview.logs.length === 0 && <li>{t("settings:diagnostics.export.noLogs")}</li>}
            </ul>
          </div>
          <p className="text-muted-foreground">{t("settings:diagnostics.export.redacted")}</p>
          <p className="text-muted-foreground">{t("settings:diagnostics.export.review")}</p>
          <pre
            className="max-h-72 overflow-auto rounded-md bg-muted px-3 py-2 font-mono text-xs leading-5 whitespace-pre select-text"
            aria-label={t("settings:diagnostics.export.content")}
            tabIndex={0}
          >
            {preview.text}
          </pre>
        </div>
      )}
    </ModalShell>
  )
}

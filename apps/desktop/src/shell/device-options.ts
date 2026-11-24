import { i18n } from "@/i18n"
import type { DeviceId, DeviceInfo, DeviceModel } from "@/lib/types"

/** A choice of the toolbar's device selector. */
export interface DeviceOption {
  /** Stable: `device:<model>:<serial>` or `model:<model>`. */
  key: string
  /** The model the editor shows for it. */
  model: string
  label: string
  /**
   * "Connected · <serial>" for a connected device, the serial of a known
   * device that is not connected; none for a model without a device.
   */
  detail?: string
  connected: boolean
  device: DeviceId | null
  /** The catalog has the model, so its layouts can be edited. */
  available: boolean
}

function modelOf(device: DeviceInfo) {
  return device.descriptor?.modelId ?? device.id.model
}

/**
 * By connection state, as the agent reports it: the connected devices
 * (named as their descriptor says), then what is not connected: the known
 * devices that are not (each by its serial) and the catalog's models
 * without any known device (to edit or simulate). A device moves between
 * the two as it connects and disconnects. A disconnected device is chosen
 * as its model, as choosing the model always was.
 */
export function deviceOptions(models: DeviceModel[], devices: DeviceInfo[]): DeviceOption[] {
  const connected = devices.filter((d) => d.state === "connected")
  const disconnected = devices.filter((d) => d.state !== "connected")
  const fromDevices = connected.map((device): DeviceOption => {
    const model = modelOf(device)
    const available = models.some((m) => m.id === model)
    return {
      key: `device:${model}:${device.id.serial}`,
      model,
      label: device.descriptor?.displayName ?? device.modelName,
      detail: available
        ? i18n.t("navigation:device.optionConnected", { serial: device.id.serial })
        : i18n.t("navigation:device.optionNotInCatalog", { serial: device.id.serial }),
      connected: true,
      device: { model, serial: device.id.serial },
      available,
    }
  })
  const fromKnown = disconnected.map((device): DeviceOption => {
    const model = modelOf(device)
    return {
      key: `device:${model}:${device.id.serial}`,
      model,
      label: device.descriptor?.displayName ?? device.modelName,
      detail: device.id.serial,
      connected: false,
      device: null,
      available: models.some((m) => m.id === model),
    }
  })
  const fromCatalog = models
    .filter((m) => !devices.some((d) => modelOf(d) === m.id))
    .map((m): DeviceOption => ({
      key: `model:${m.id}`,
      model: m.id,
      label: m.name,
      connected: false,
      device: null,
      available: true,
    }))
  return [...fromDevices, ...fromKnown, ...fromCatalog]
}

/**
 * The option for what the editor shows: the device it was chosen through,
 * else a connected device of the model, else the model.
 */
export function currentOption(
  options: DeviceOption[],
  model: string | null,
  device: DeviceId | null,
): DeviceOption | undefined {
  if (!model) return undefined
  return (
    (device &&
      options.find((o) => o.device?.serial === device.serial && o.model === device.model)) ||
    options.find((o) => o.model === model)
  )
}

/**
 * The connected device the editor is showing, as the agent reports it: the
 * one it was chosen through, else a connected device of the model.
 */
export function selectedDevice(
  devices: DeviceInfo[],
  model: string | null,
  device: DeviceId | null,
): DeviceInfo | null {
  if (!model) return null
  const connected = devices.filter((d) => d.state === "connected" && modelOf(d) === model)
  return (
    (device && connected.find((d) => d.id.serial === device.serial && device.model === model)) ||
    connected[0] ||
    null
  )
}

// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { DRAG_TYPE, flowEntries } from "./catalog"
import { CatalogPanel } from "./catalog-panel"

const groups = [{ id: "flow", title: "Flow", entries: flowEntries(undefined), types: [] }]

describe("node catalog", () => {
  afterEach(cleanup)

  it("drags entries with their key and adds them on click or keyboard", () => {
    const onAdd = vi.fn()
    render(<CatalogPanel groups={groups} onAdd={onAdd} onCreateAction={vi.fn()} />)
    const delay = screen.getByRole("listitem", { name: /Delay/ })
    const setData = vi.fn()
    fireEvent.dragStart(delay, { dataTransfer: { setData, effectAllowed: "" } })
    expect(setData).toHaveBeenCalledWith(DRAG_TYPE, "flow:delay")
    // A button: Enter and Space activate it like a click.
    fireEvent.click(delay)
    expect(onAdd).toHaveBeenCalledWith(expect.objectContaining({ key: "flow:delay" }))
    expect(delay.tagName).toBe("BUTTON")
  })

  it("filters by search", () => {
    render(<CatalogPanel groups={groups} onAdd={vi.fn()} onCreateAction={vi.fn()} />)
    fireEvent.change(screen.getByLabelText("Search nodes"), { target: { value: "wait" } })
    expect(screen.getAllByRole("listitem").map((e) => e.textContent)).toEqual([
      expect.stringContaining("Join"),
      expect.stringContaining("Delay"),
    ])
    fireEvent.change(screen.getByLabelText("Search nodes"), { target: { value: "zzz" } })
    expect(screen.getByText("No matching nodes")).toBeTruthy()
  })

  it("never cuts a long description or reason to one line, and shows it whole on hover", () => {
    const long =
      "Stops and removes a Compose project's containers and networks. Volumes and images are kept. Needs the Docker plugin's permission to start, stop and restart containers."
    const [first] = flowEntries(undefined)
    if (!first) throw new Error("no flow entry")
    const docker = [
      {
        id: "docker",
        title: "Docker",
        entries: [
          { ...first, key: "docker:down", title: "Compose down", description: long },
          {
            ...first,
            key: "docker:up",
            title: "Compose up",
            description: "",
            unavailable:
              "Docker is not reachable in this context; start Docker Desktop or choose another context.",
          },
        ],
        types: [],
      },
    ]
    render(<CatalogPanel groups={docker} onAdd={vi.fn()} onCreateAction={vi.fn()} />)
    const down = screen.getByRole("listitem", { name: /Compose down/ })
    const text = down.querySelector("[data-slot=catalog-description]")
    expect(text?.textContent).toBe(long)
    expect(text?.className).toContain("line-clamp-2")
    expect(text?.className).not.toContain("truncate")
    expect(down.getAttribute("title")).toContain(long)
    const up = screen.getByRole("listitem", { name: /Compose up/ })
    expect(up.textContent).toContain("start Docker Desktop or choose another context.")
    expect(up.querySelector(".truncate.text-xs")).toBeNull()
  })
})

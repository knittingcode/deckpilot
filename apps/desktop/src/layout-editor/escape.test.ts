// @vitest-environment jsdom
// Escape in the Device Editor: one effect per press (escape.ts).
import { afterEach, describe, expect, it } from "vitest"

import { escapeEffect } from "./escape"

afterEach(() => {
  document.body.innerHTML = ""
})

const press = (target: Element | null, defaultPrevented = false) => ({
  key: "Escape",
  defaultPrevented,
  target,
})

describe("escapeEffect", () => {
  it("deselects from a key, a button or a switch", () => {
    document.body.innerHTML = `<button id="key"></button><button role="switch" id="s"></button>`
    expect(escapeEffect(press(document.getElementById("key")), false)).toBe("deselect")
    expect(escapeEffect(press(document.getElementById("s")), false)).toBe("deselect")
    expect(escapeEffect(press(document.body), false)).toBe("deselect")
    expect(escapeEffect({ ...press(document.body), key: "Enter" }, false)).toBeNull()
  })

  it("does nothing while an overlay is open or the press came from one", () => {
    document.body.innerHTML = `<div role="menu"><button id="item"></button></div>`
    expect(escapeEffect(press(document.getElementById("item")), false)).toBeNull()
    document.body.innerHTML = `<button id="key"></button><div data-radix-popper-content-wrapper></div>`
    expect(escapeEffect(press(document.getElementById("key")), false)).toBeNull()
    document.body.innerHTML = `<button id="key"></button><div role="alertdialog" data-state="open"></div>`
    expect(escapeEffect(press(document.getElementById("key")), false)).toBeNull()
  })

  it("does nothing during a drag or when something else took the press", () => {
    document.body.innerHTML = `<button id="key"></button>`
    const key = document.getElementById("key")
    expect(escapeEffect(press(key), true)).toBeNull()
    expect(escapeEffect(press(key, true), false)).toBeNull()
  })

  it("leaves a text field instead of deselecting", () => {
    document.body.innerHTML = `
      <input id="text" />
      <input id="search" type="search" />
      <textarea id="area"></textarea>
      <div id="edit" contenteditable="true"></div>
      <input id="check" type="checkbox" />`
    for (const id of ["text", "search", "area"]) {
      expect(escapeEffect(press(document.getElementById(id)), false), id).toBe("leaveField")
    }
    expect(escapeEffect(press(document.getElementById("check")), false)).toBe("deselect")
  })
})

import { describe, expect, it } from "vitest"

import { clip, inertText, safeHttpsUrl } from "./text"

describe("storefront text", () => {
  it("removes bidi overrides, isolates, zero-width and control characters", () => {
    expect(inertText("evil\u202Etxt.exe")).toBe("eviltxt.exe")
    expect(inertText("zero\u200Bwidth and \u2066isolate\u2069")).toBe("zerowidth and isolate")
    expect(inertText("a\u0000b\u0007c\u009Fd\uFEFF")).toBe("abcd")
    expect(inertText("one\ntwo")).toBe("one two")
    expect(inertText("one\ntwo", true)).toBe("one\ntwo")
    expect(inertText(42)).toBe("")
    // Tags, soft hyphens, separators, Hangul fillers, variation selectors and the rest.
    const invisible = [
      "\u{E0001}",
      "\u{E0041}",
      "\u{E007F}",
      "\u00AD",
      "\u2028",
      "\u2029",
      "\u115F",
      "\u1160",
      "\u3164",
      "\uFFA0",
      "\uFE00",
      "\uFE0F",
      "\u{E0100}",
      "\u{E01EF}",
      "\u034F",
      "\u180E",
      "\u061C",
      "\u200E",
      "\u200F",
      "\uFFF9",
      "\uFFFA",
      "\uFFFB",
      "\u206A",
      "\u206F",
      "\u17B4",
      "\u17B5",
      "\u{1D173}",
      "\u{1D17A}",
      "\u{1BCA0}",
      "\u{1BCA3}",
    ]
    for (const ch of invisible) {
      expect(inertText(`De${ch}ck${ch}Pilot`), `U+${ch.codePointAt(0)?.toString(16) ?? ""}`).toBe(
        "DeckPilot",
      )
    }
    expect(inertText(invisible.join(""))).toBe("")
    // Visible text, emoji and Polish letters stay.
    expect(inertText("Zażółć gęślą jaźń 😀")).toBe("Zażółć gęślą jaźń 😀")
    // Markup is only characters.
    expect(inertText("<script>alert(1)</script>")).toBe("<script>alert(1)</script>")
  })

  it("clips by code points with an ellipsis", () => {
    expect(clip("W".repeat(6000), 120)).toHaveLength(120)
    expect(clip("W".repeat(6000), 120).endsWith("…")).toBe(true)
    expect(clip("short", 120)).toBe("short")
    // A surrogate pair is never split.
    expect(Array.from(clip("😀".repeat(10), 5))).toHaveLength(5)
  })

  it("accepts only https links by the storefront's rules", () => {
    for (const ok of [
      "https://github.com/knittingcode/deckpilot-plugins",
      "https://acme.example/help?x=1#y",
      "https://acme.example/v1.2/notes...md",
      "https://a1.example/x",
      // Numeric and hex-like labels are ordinary unless last (real hosts).
      "https://163.com/",
      "https://360.cn/",
      "https://123.example.com/",
      "https://www.1.com/",
      "https://0x.co/",
      "https://0xabc.dev/",
    ]) {
      expect(safeHttpsUrl(ok)).toBe(ok)
    }
    for (const bad of [
      "javascript:alert(1)",
      "data:text/html;base64,PHNjcmlwdD4=",
      "http://acme.example/",
      "https://user:pw@acme.example/",
      "https://acme.example:8443/",
      "https://127.0.0.1/",
      "https://[::1]/",
      "https://xn--80ak6aa92e.com/",
      "https://ACME.example/",
      "https://localhost/",
      "https://acme.example/\u202Egnp.exe",
      "https://acme.example/a b",
      `https://acme.example/${"a".repeat(520)}`,
      "//acme.example/",
      // IP literals in every form.
      "https://0x7f.1/",
      "https://0177.0.0.1/",
      "https://127.1/",
      "https://2130706433/",
      "https://acme.0x10/",
      "https://0x7f.0.0.1/",
      "https://10.0.0.1/",
      "https://acme.123/",
      "https://[2001:db8::1]/",
      "https://[::ffff:127.0.0.1]/",
      // Dot segments, plain or encoded.
      "https://acme.example/a/../b",
      "https://acme.example/./b",
      "https://acme.example/a/..",
      "https://acme.example/a/%2e%2e/b",
      "https://acme.example/a/.%2E/b",
      "https://acme.example/%2e/b",
      null,
    ]) {
      expect(safeHttpsUrl(bad)).toBeNull()
    }
  })
})

# Key text: wrapping, fitting and scrolling titles

How a key's title is laid out on a Stream Deck key, in the device editor's
preview and on the device. There is **one implementation**: the shared
renderer (`crates/deckpilot-render`, `face.rs` and `text.rs`). The agent
draws device images with it, and the app's preview shows images the same
renderer drew (`preview_layout`), so preview and hardware agree by
construction. Plugins and modules only provide text (a label, a detail);
they never lay it out.

## Settings

Every binding's appearance can carry a **text appearance**
(`ControlBinding.appearance.text`, `deckpilot_core::TextAppearance`). All
fields are optional; a text appearance that sets nothing is not stored.

| Field | Values | Default | Used by |
| ----- | ------ | ------- | ------- |
| `mode` | `wrap`, `singleLine`, `autoFit`, `marquee` | `wrap` | all |
| `size` | preferred size in points, 6–32 | the appearance's `fontSize` (small 9.4, medium 11.2, large 13.7 pt) | all |
| `align` | `left`, `center`, `right` | `center` | all (title and value lines) |
| `maxLines` | 1–6 | automatic: 2 under an icon, 3 without (one fewer with a two-line value) | `wrap`, `autoFit` with `fit: multiline` |
| `overflow` | `ellipsis`, `clip` | `ellipsis` | `singleLine` |
| `minSize` | smallest size in points, 6–32 (never above the preferred size) | 7 | `autoFit` |
| `fit` | `singleLine`, `multiline` | `singleLine` | `autoFit` |
| `speed` | device pixels per second, 5–240 | 30 (presets in the app: slow 15, normal 30, fast 60) | `marquee` |
| `pause` | milliseconds at the start and at the end, 0–10 000 | 1000 | `marquee` |

**Points** are pixels on a 72 px key (Stream Deck MK.2) and scale with the
key's shorter side: 11 pt is 11 px on an MK.2, 14.7 px on a 96 px XL key,
18.3 px on a 120 px Stream Deck + key, and 15.3 px in a 200 × 100 touch
strip zone. So a layout looks the same on every model, and the rules below
give the same lines at every key size (tested for every key size of the
device catalogue, the Mini's 80 px, and previews drawn at twice the size).

Values out of range are refused when a layout is saved in the app; the
renderer clamps them anyway (e.g. in a hand-edited import).

Fields of another mode are kept (switching modes back and forth keeps
them) but not used.

## Bounds and font

- **Key size** comes from the device descriptor: the model's key image
  (`DeviceCapabilities.keys.image`) or touch zone image; nothing assumes
  one LCD size.
- **Text area**: the key width less a padding of 7 % of the shorter side
  (at least 2 px) on each side. Vertically: the key height less a margin of
  1/12 of the height (at least 3 px) at the top and bottom, which keeps the
  toggle and progress marks free. The value line (the face's detail) and an
  icon or glyph above the title share that height.
- **Font**: Inter Medium (titles) and SemiBold (values), compiled into the
  renderer; widths include kerning, the line height is ascent to descent.
  A bold title uses SemiBold, an italic one is slanted (see [Title
  appearance](#title-appearance)).
- **Characters**: one glyph per grapheme cluster (its first character): a
  letter with combining marks shows the letter, an emoji sequence one `?`
  (the font has no emoji). Polish letters (ąćęłńóśźż ĄĆĘŁŃÓŚŹŻ) are in the
  font. Text is only ever cut **between grapheme clusters**.

## Title appearance

Every key with a display that does something (an action, a Multi Action, a
plugin's action, a layout navigation key, a folder key) has **one title
editor**, the same for all of them: the **T▾** button next to the key's
title at the top of the Inspector opens a popover (a portal above the
Inspector and its curtain, kept inside the window, not modal) with

1. position **Top / Middle / Bottom** · **Reset**,
2. the **font**,
3. the **size** in px (points, see above) · **B / I / U** · the **text colour**,
4. the **text layout** (the modes below) · alignment, and the mode's own
   settings (lines, overflow, minimum size, speed, pause).

The folder's protected Back key has no title and no editor. The header is
the same for every such key: on the left the key's live preview (the image
the Canvas shows: the same renderer at the same size; clicking it opens the
key menu and never runs the key or opens its folder); on the right the
visible title with **T▾**, and under it the **Show title** switch (hiding
keeps the title, its look and a folder's name; showing restores them).
The visible title is the label of the binding the key shows; for a **folder
key** it is the folder page's name, its one title (see [Folder
names](#folder-names)). Icon and background are the key menu's (see [Icons
and backgrounds](#icons-and-backgrounds)).

| Field (`appearance`) | Values | Default |
| ----- | ------ | ------- |
| `showTitle` | `false` hides the title and the value line; the title and its look are kept | shown |
| `titleAlign` | `top`, `middle`, `bottom` | a plugin's suggestion, else bottom under an icon, middle without |
| `fontFamily` | a font of `deckpilot_core::TITLE_FONTS` (the app asks `title_fonts`): today `inter` | `inter` |
| `bold` | `true`, `false` | not bold |
| `italic` | `true`, `false` | upright |
| `underline` | `true`, `false` | not underlined |
| `textColor` | `#rrggbb` | the model's `textColor` (white on an MK.2) |
| `text.size` | 6–32 px on a 72 px key | the `fontSize` preset |

**Unset is not explicit**: every field is stored only when the user sets
it; `false` is kept (it beats a suggestion). **Reset** clears the
formatting (whether the title shows, the background and the icon stay); it
never writes the resolved defaults.

### Folder names

A key that opens a folder shows the folder page's name (`page.name`): one
name, edited in the key header, read by the key, the Canvas, the
breadcrumbs, the Page Navigator and every reference. The folder key's
binding stores no title of its own; the Rust resolver (`device_runtime`'s
face of an `OpenPage` to a folder) and the app's (`folderKeyTitle`) derive
it when drawing. **Show title** and the title's look apply to the key only.
Layouts from before (schema 12 and older, older exports) could give a
folder key a title of its own: migration 13 (after the encrypted backup)
and imports drop one equal to the name and keep one that differs in the
layout's `recovered` list (`reason: "folderLabel"`), with a migration note
or an import warning; the folder key's Inspector offers it as **Use as
folder name** or **Discard**. Saving does the same, so nothing typed is
ever lost silently.

### Icons and backgrounds

Clicking the key preview opens the **key menu**, the same for every
configurable key, in two sections:

- **Icon**: *Choose from icon library…*, *Import icon from file…*, *Paste
  icon from clipboard*, *Remove icon* (when the key has one);
- **Background**: *Choose background…*, *Choose custom colour…* (the
  colour field: palette, the system colour picker, `#rrggbb`), *Reset to
  device default* (when the key sets one).

A menu item closes the menu, then opens its picker in one popover anchored
to the preview (kept inside the window, scrolling only inside its grid);
Escape closes it and the focus goes back to the preview. Choosing a tile is
an undo step of the draft, which the live preview shows; nothing runs.

**The Icon Library** has three sources in one shared library: **DeckPilot**
(471 Lucide icons in 19 categories, built into the renderer's icon font,
searchable by name and Lucide's search words, offline, always there),
**My Icons** (the profile's imported images) and **Plugins** (icons of
installed plugins, `contributes.visualAssets`). **The Background Library**
shows **Transparent, Black and White** first (always there), then solid
colours, DeckPilot's gradients, images (My Icons) and plugin backgrounds.
Tiles are drawn by the keys' renderer; arrow keys move between them, Home
and End to the ends.

Keys store a reference, never image data:

| `appearance` field | Values |
| ----- | ------ |
| `icon.id` | a profile image's id (UUID, as before), `deckpilot.<lucide-name>`, or `<plugin id>.<id>` |
| `background` | unset (the device's default), `transparent`, `#rrggbb`, or `asset:<ref>`: `asset:deckpilot.gradient-<name>`, `asset:<plugin id>.<id>`, `asset:<profile image id>` |

Unset, explicit transparent, explicit black and explicit white stay
distinct. A DeckPilot icon is drawn from the font as the key's mark, in its
text colour, on whatever background it has (transparency kept); an image
icon covers the key by its fit as before. A gradient or an image background
is rasterised by the renderer (gradients as CSS `linear-gradient` angles,
images cropped to cover the key) over the key's default background; a
background asset is the key's own choice, so it beats a state's accent.
Precedence is unchanged otherwise.

A reference to an asset that is not there — a disabled or uninstalled
plugin, an id of a later version — is kept as it is: the key draws no icon
of its own and the device's default background, the pickers say the asset
is unavailable, and the key draws it again as soon as the plugin is back.
Nothing rewrites a layout for it. An image a key uses as its background
counts as used (it cannot be deleted) and travels with exports like an icon;
DeckPilot's own assets need nothing; plugin assets travel as references.

**Fonts**: only fonts built into the renderer can be chosen, so the device,
the Canvas and every model draw them alike; the list comes from one place
(`TITLE_FONTS`, checked against the renderer's faces by a test). DeckPilot
bundles Inter Medium and Inter SemiBold only, so:

- **bold** is Inter SemiBold (there is no heavier face);
- **italic** is Inter slanted by 10° (there is no italic face; Inter's own
  italic leans 9.4°). The slant's overhang is part of the measured width,
  so wrapping, `…`, auto fit and scrolling keep italic text inside the text
  area;
- **underline** is a line at the font's underline position and thickness
  (at least 1 px), as wide as the text, clipped and scrolled with it.

A font id the renderer does not know (a later version's) is kept in the
layout and drawn with Inter; saving it from the app is refused.

### Title precedence

What a key shows as its title, first match wins:

1. the key's own title (the binding's `label`);
2. the action's own label;
3. for *Switch layout*, the layout's name;
4. a plugin's **dynamic title** (its state's `label`) — for plugin actions
   only *after* 1 and 2: a title the user wrote is never replaced by a
   plugin. Built-in modules' states keep replacing the title, as before;
5. a plugin's **suggested title** (`title.default`), while the action still
   has its type's name;
6. the action's name.

Whichever wins is a display template (`docs/variables.md` §3): 1, 2, 3, 5
and 6 may hold references, a plugin's suggested title included, and are
resolved with the display-safe variables only. A dynamic title (4) is the
plugin's text as it sent it, never resolved.

### Plugin titles

A plugin's action may declare (plugin API 1.8,
`contributes.actions[].title`, data only, no permission):

```json
"title": {
  "default": "CPU",
  "dynamic": true,
  "position": "bottom",
  "supports": ["position", "color", "layout"]
}
```

`default` and `position` are suggestions under the user's own values (the
position fills in only while the key sets none; nothing is written into the
layout). `dynamic` says the action sets its title while it runs. `supports`
lists the parts of the look the editor offers for its keys (`visibility`,
`position`, `font`, `style`, `color`, `layout`; absent: all); the others are
shown disabled. A plugin's default icon is its action's `node.icon`, under
the key's own icon. The editor is always DeckPilot's: a plugin cannot
replace it or draw its keys itself.

## Dynamic text

Templates (`{project.key}`, `{project.name}`, `{profile}`, `{page.…}`, and
references such as `{{ shared.team.name }}`, see `docs/variables.md` §3) are
resolved by the agent, and a module's or plugin's label and detail applied, **before**
the title is measured. A different resolved text is a different face, so it
is laid out (fitted, wrapped) again; frames are cached by face and size, so
an unchanged text is not drawn twice.

## Modes

### Wrap (default)

- Lines break between words (any whitespace), greedily.
- A word wider than a line on its own is broken between grapheme clusters,
  as late as possible. Words that fit are never broken.
- An explicit line break (`\n`) always ends a line; empty lines between
  paragraphs are kept, leading and trailing ones dropped.
- At most `maxLines` lines. When text is left, the last line holds the rest
  of the text (line breaks read as spaces there) cut with `…`.
- Never overflows: with `maxLines` set, only as many lines as fit the key's
  height are used; without it (automatic), lines that do not fit are left
  out, and a second line gives way to an icon when the icon would get
  smaller than 30 % of the key.
- Without a text appearance this is exactly how keys were drawn before
  text modes existed (checked against the previous algorithm for every
  width and line count), except that a title containing a line break now
  breaks there (labels typed in the app have none).

### Single line

- One line (line breaks read as spaces) at the preferred size. It is never
  made smaller.
- `ellipsis`: cut with `…`. `clip`: drawn from the start of the text area
  and clipped at its edges (an alignment does not apply to a clipped line).

### Auto fit

- Starts at the preferred size and only shrinks, never enlarges, down to
  `minSize` (in steps of 1/48 of the range, at least 0.25 px).
- `fit: singleLine`: the largest size at which the whole line fits the
  width (and the height, leaving room for an icon).
- `fit: multiline`: the largest size at which the text wraps (by the wrap
  rules) into at most `maxLines` lines **without** `…` and **without**
  breaking a word, within the height.
- If nothing fits at `minSize`, the title is not clipped silently: it is
  drawn at `minSize` by the wrap rules (single line: cut with `…`), and
  the report says `fits: false`; the Inspector then shows *“Doesn't fit at
  the minimum size”*.

### Marquee

- One line at the preferred size. A title that fits is drawn still, by
  the alignment, and never animated.
- A wider title scrolls on the device: `distance` = its width less the
  text area's, rounded up to whole pixels.
- **Offset** (pixels to the left), a pure function of the time `t` since
  the animation started, with `pause` *P* and speed *v*:
  - cycle = 2 *P* + `distance` / *v*; *t* is taken modulo the cycle;
  - *t* < *P*: 0 (the start is shown);
  - then ⌊(*t* − *P*) · *v*⌋, up to `distance` (computed in whole numbers,
    so the moment each pixel is reached is exact);
  - the end pause: `distance`; then back to 0 in one step.
- Frames: the next frame is due when the offset changes next, but never
  sooner than 1/15 s after the previous one (`MARQUEE_FPS` = 15,
  `deckpilot_render::next_frame`). A pause is one frame. Fast speeds move
  several pixels a frame; the end may be reached up to one frame late.
- Drawn clipped to the text area. Still (the app's preview with reduced
  motion, and the `image` of `preview_layout`): cut with `…`.

## Layout report

`deckpilot_render::text_report(face, width, height, icons)` lays a key out
without drawing it. The app's `preview_layout` (with `render`) returns it
for every control with a title as `text`, **measured at the device's own
key size** (the image may be drawn larger for a sharp preview):

```json
{ "mode": "autoFit", "fits": false, "truncated": true, "fontSize": 7, "lines": 1, "needsScroll": false }
```

`fits`: all of the title is shown (a scrolling title shows it in turn);
`truncated`: drawn cut (`…`, clipped, lines left out); `fontSize` in points
(to a tenth); `needsScroll`: it scrolls on the device. The field is new and
optional, so older callers ignore it.

`preview_marquee` returns one cycle of a scrolling key's frames, timed as
on the device (`{ frames: [{ image, durationMs }], cycleMs }`), drawn at
the preview size with the offsets scaled from the device's pixels. The
device editor plays them on the key, unless the system asks for reduced
motion: then the key shows its still image (cut with `…`) and the Inspector
says it scrolls on the device.

## Scrolling on hardware

The agent (`apps/agent/src/marquee.rs`):

- **Which keys**: on every redraw of a device, the keys of the page that
  device shows whose title is a marquee title wider than the key, at the
  device's key size. Keys of other pages, of disconnected devices, and
  everything while locked (the keys show *Locked*) never animate.
- **One animator thread per connected device**, started with the device's
  session and stopped (joined) when it disconnects; reconnecting makes the
  list again. It sleeps until the next frame is due, and blocks without a
  timeout while nothing scrolls. It renders at the device's native key
  resolution and sends only frames that differ from the one it sent last
  (no USB write for an unchanged frame).
- **Never blocks input**: rendering happens on the animator thread; frames
  go to the device's I/O thread through its queue, written between input
  reads like any image. While frames arrive the read timeout is 30 ms
  instead of 100 ms, so they are not held back.
- **Stops and restarts**: a page switch, a layout, profile or icon change,
  locking or a lock-screen placeholder gives a new list (an unchanged key
  keeps its place in the cycle, a changed one starts again). Each list is a
  new **epoch**: the epoch is advanced before the still images of the new
  page are queued, and the device thread drops frames of an older epoch,
  so a frame already on its way never draws over the new page.
- **Clock**: an injectable monotonic clock (`Agent::with_marquee_clock`);
  the scheduler (`MarqueeScheduler`) is a pure function of the times it is
  given and is tested with explicit times.

### Cost

Measured by `the_cost_of_a_scrolling_key_is_bounded`
(`crates/deckpilot-render/src/face/text_modes.rs`; run with
`cargo test --release -p deckpilot-render cost -- --nocapture`): one
scrolling key with an icon, defaults (30 px/s, 1 s pauses), rendered and
encoded as JPEG at quality 90 as the HID driver sends it, on an Apple
Silicon Mac:

| Key | Frames per cycle | Average frames/s (15 while moving, 0 in the pauses) | Render + JPEG per frame | Data to the device |
| --- | --- | --- | --- | --- |
| 72 px (MK.2) | 89 in 7.8 s | 11.4 | 0.18 ms | 21 KB/s |
| 96 px (XL) | 118 in 9.7 s | 12.1 | 0.17 ms | 33 KB/s |
| 120 px (+) | 147 in 11.7 s | 12.6 | 0.17 ms | 45 KB/s |

So a page with all 15 MK.2 keys scrolling costs about 3–4 % of one core
and about 320 KB/s, well within USB full speed; a key whose title fits costs
nothing.

## Labels

A key can show up to **8 labels** besides its title: short texts placed
anywhere on the key, each with its own size, colour, plate and layer
(`KeyAppearance.labels`, `deckpilot_core::appearance::labels`). They live on
the look of the binding the key shows (press, else click, else the first),
like the title and the icon; labels on another trigger's look are kept but
not drawn, and `DeviceLayout::warnings` reports them (`DormantLook`).

**Text.** A label's `text` is a display template of at most 160 characters,
resolved exactly like a title (`resolve_display_template`: `{{ reference }}`,
the older `{profile}`, `{project.key}`, `{project.name}`, then `{page.…}`).
Only display-safe sources resolve; a reference without a displayable value
shows `-`. A template that names a secret, a sensitive value or a run's
value (`steps.…`, `workflow.…`) is refused when saved or imported, by the
core and, while the agent runs, by the agent's own variables: a plugin's or
module's variable declared sensitive or an identifier, or a shared variable
whose value is sensitive, is refused too. A reference the agent does not
know yet (a plugin that is not installed) is kept and shows `-` until its
plugin declares it display-safe. `{page.…}` is expanded in the template
before references resolve, so a value is never expanded. At most 120
grapheme clusters of the resolved text are laid out (then `…`).
`visible` hides a label while its references are unresolved or its text is
empty (`whenResolved`) or while no project is active (`whenProject`);
`enabled: false` keeps a label without drawing it.

**Units.** Labels are laid out on a canvas whose shorter side is 72 units
(a 200×100 touch zone is 144×72), the unit being the title's point. Line
breaks, cuts and boxes are computed in units and only scaled to pixels when
drawn, so a label breaks the same way at 72, 96, 120 px and in a 2× or 3×
preview. Plate edges are snapped to whole pixels.

**Position.** `preset` (top, center, bottom or a corner of the text area,
5 units in from the sides and 6 from the top and bottom), `anchored` (an
edge or corner of the key itself, `inset` units in: ribbons and corner
tags) or `absolute` (`x`, `y` per mille of the canvas, `origin` the point of
the label's box placed there). `offsetX`/`offsetY` move it by units. Width
is fixed (`width`, percent) or follows the text up to `maxWidth` (presets at
top, center and bottom and edge anchors: the full width; corners: 60 %);
height likewise (`height`, `maxHeight`). The box is moved, never cut, to
stay on the key. Text taller than a fixed height is clipped to the box.

**Overflow.** `ellipsis` (one line, `…`, default), `clip`, `wrap` (up to
`maxLines`, 1–4, default 2, the last line cut with `…`; lines that do not fit
the height are dropped) and `shrink` (one line, down to `minSize`, default 6,
then `…`). No scrolling labels.

**Layers.** Labels float: they never move the title, the mark or the icon,
and may overlap them on purpose (the layout report says when a label
overlaps the title). The paint order is: background, background asset,
`behindIcon` labels, icon image, series, Back icon or mark, `overIcon`
labels (default), the title (with its scrim) and value lines, `overTitle`
labels, the highlight border, then the state marks (badge, toggle, progress,
indicator), always on top. Within a layer, labels are painted in list order.

**Plate.** `background: auto` (default) draws a plate when the label's box
meets a picture (the icon image, a background image or gradient, a series),
the title, the key's icon mark or a state mark (badge, toggle, progress,
indicator), or when its colour has less than 4.5:1 contrast with the key's
background. A label that sets no layer and meets the title is painted over
the title, on its plate (under the title the plate could not make either
readable); a layer the label sets is always kept. The plate is
black at `backgroundOpacity` (default 60 %) behind light text, white behind
dark text. `none` never draws one; `#rrggbb` draws that colour (default
100 %). An unset colour is the key's text colour, or white or black where
that does not read on the plate. Decided from the layers, not by sampling
pixels, so it is the same at every size.

**Size.** 6–32 points (default 10); `weight` regular (Inter Medium) or bold
(SemiBold); `align` defaults by position (left in left corners, right in
right ones, else centred); `padding` and `radius` 0–12 units (default 2).

**Redraws.** Each label is its own text slot (`TextSlot::Label { id }`), so
the agent knows what each label of each shown binding depends on. A change of
a variable no shown text uses redraws nothing. Variable-driven redraws are
coalesced per device: the first change redraws 50 ms later, further changes
ride along, and a device is redrawn for variables at most 20 times a second
(a change shows within about 100 ms). Input, actions, profile, layout and
page changes still redraw at once, and the run loop never waits for a
redraw: a key press during a storm of changes is handled as fast as without
it (`apps/agent/tests/key_labels.rs`). A redraw over 8 ms is logged with its
key count, never its texts.

**Cost.** `cargo test --release -p deckpilot-render --test label_cost --
--nocapture` measures it (Apple M-series, render plus JPEG): the worst-case
key (8 labels over an icon image, with plates) in every overflow mode at
72, 96 and 120 px is at most about 0.85 ms (wrapping on 4 lines at 120 px;
budget 1 ms), a label text change about 0.09 ms at 72 px
(budget 0.3 ms; icon and background images are scaled once per size and
kept, `ScaledImageCache`), against about 0.05 ms for the same key without
labels.

The renderer is the only place labels are laid out: the app's previews are
the renderer's images, and the editor gets each label's box from it
(`labelBoxes`).

## Compatibility

The title appearance fields (`fontFamily`, `bold`, `italic`, `underline`)
are additive and optional too: a look without them renders exactly as
before (tested pixel for pixel), no migration, no IPC protocol change
(the action types' `title` is an optional field). Older agents and apps
ignore them; an older app drops them when it saves the layout. Plugin
manifests using `title` need plugin API 1.8.

`text` is an additive, optional field of the appearance: no schema or
document version change, no migration, and no IPC change (layouts reach
the agent through the database). Older agents ignore it and draw wrapped
titles. Exports keep it (payload version unchanged) and older versions
import them with wrapped titles. An older *app* does not know the field
and drops it when it saves that layout. See [compatibility.md](compatibility.md).

`labels` is additive and optional too: an empty list is not written, and a
key without labels renders byte for byte as before (the G0 golden images,
`crates/deckpilot-render/tests/golden`). Older agents ignore labels and draw
the key without them; an older app drops them when it saves that layout.
Exports with labels are written as payload version 4 (older versions refuse
them instead of importing them without labels); exports without labels keep
their version. Agents that draw labels announce the IPC capability
`keyLabels`.

---
name: Whetstone
description: The rulebook drawn as a controlled engineering document, one framed sheet with a title block, numbered requirements and a revision table.
colors:
  desk: "#e9e6df"
  sheet: "#ffffff"
  ink: "#0a0a0a"
  ink-2: "#47443f"
  ink-3: "#6b675f"
  rule: "#d8d4cb"
  frame: "#0a0a0a"
  card: "#f6f4ef"
  muted: "#f1eee8"
  muted-foreground: "#5c5850"
  input: "#c9c4ba"
  primary: "#e66c3a"
  primary-hover: "#d95f2d"
  primary-foreground: "#0a0a0a"
  struck: "#c2410c"
  wash: "#fdf1eb"
  ring: "#e66c3a"
  stone-top: "#e66c3a"
  stone-left: "#c2410c"
  stone-right: "#d95f2d"
  dark-desk: "#050506"
  dark-sheet: "#0b0c0f"
  dark-ink: "#ece8df"
  dark-ink-2: "#bdb7ac"
  dark-ink-3: "#928c81"
  dark-rule: "#262830"
  dark-frame: "#8d877c"
  dark-card: "#121419"
  dark-muted: "#15171d"
  dark-muted-foreground: "#a8a297"
  dark-accent: "#1a1d24"
  dark-input: "#343741"
  dark-primary-hover: "#ef7d4e"
  dark-primary-foreground: "#0b0c0f"
  dark-struck: "#ef7d4e"
  dark-wash: "#1f1510"
typography:
  purpose:
    fontFamily: "Vollkorn, Georgia, serif"
    fontSize: "1.5rem"
    fontWeight: 500
    lineHeight: 1.25
  headline:
    fontFamily: "Oswald, 'Arial Narrow', sans-serif"
    fontSize: "1.375rem"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "0.04em"
  title:
    fontFamily: "Oswald, 'Arial Narrow', sans-serif"
    fontSize: "1.125rem"
    fontWeight: 500
    letterSpacing: "0.04em"
  wordmark:
    fontFamily: "Oswald, 'Arial Narrow', sans-serif"
    fontSize: "1.0625rem"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "0.22em"
  statement:
    fontFamily: "Vollkorn, Georgia, serif"
    fontSize: "1.0625rem"
    fontWeight: 500
    lineHeight: 1.375
  body:
    fontFamily: "Vollkorn, Georgia, serif"
    fontSize: "16px"
    fontWeight: 400
    lineHeight: 1.55
    fontFeature: "'lnum' 1, 'tnum' 1"
  body-small:
    fontFamily: "Vollkorn, Georgia, serif"
    fontSize: "0.9375rem"
    fontWeight: 400
    lineHeight: 1.55
  number:
    fontFamily: "Oswald, 'Arial Narrow', sans-serif"
    fontSize: "0.8125rem"
    fontWeight: 600
    letterSpacing: "0.04em"
  button:
    fontFamily: "Oswald, 'Arial Narrow', sans-serif"
    fontSize: "0.8125rem"
    fontWeight: 500
    letterSpacing: "0.08em"
  label:
    fontFamily: "Oswald, 'Arial Narrow', sans-serif"
    fontSize: "0.6875rem"
    fontWeight: 500
    lineHeight: 1.2
    letterSpacing: "0.1em"
  mono:
    fontFamily: "'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, Consolas, monospace"
    fontSize: "0.8125rem"
rounded:
  none: "0px"
spacing:
  hairline: "1px"
  cell-x: "12px"
  cell-y: "8px"
  band-x: "16px"
  band-x-sm: "24px"
  callout: "20px"
  group: "40px"
  section: "48px"
  part: "64px"
  sheet-margin: "24px"
  desk-margin: "32px"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.primary-foreground}"
    typography: "{typography.button}"
    rounded: "{rounded.none}"
    height: "36px"
    padding: "8px 16px"
  button-primary-hover:
    backgroundColor: "{colors.primary-hover}"
  button-outline:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    typography: "{typography.button}"
    rounded: "{rounded.none}"
    height: "32px"
    padding: "0 12px"
  button-outline-hover:
    backgroundColor: "{colors.muted}"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.button}"
    rounded: "{rounded.none}"
    height: "32px"
    padding: "0 12px"
  button-destructive:
    backgroundColor: "transparent"
    textColor: "{colors.struck}"
    typography: "{typography.button}"
    rounded: "{rounded.none}"
    height: "36px"
    padding: "8px 16px"
  button-destructive-hover:
    backgroundColor: "{colors.wash}"
  input:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    rounded: "{rounded.none}"
    height: "36px"
    padding: "4px 12px"
  title-block-cell:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    typography: "{typography.body-small}"
    rounded: "{rounded.none}"
    padding: "8px 12px"
  chip:
    backgroundColor: "transparent"
    textColor: "{colors.ink-2}"
    typography: "{typography.label}"
    rounded: "{rounded.none}"
    height: "20px"
    padding: "0 6px"
  chip-strong:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.none}"
    height: "20px"
    padding: "0 6px"
  pending-band:
    backgroundColor: "{colors.wash}"
    textColor: "{colors.ink}"
    rounded: "{rounded.none}"
    padding: "12px 24px"
  callout:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    rounded: "{rounded.none}"
    padding: "20px"
  command:
    backgroundColor: "{colors.card}"
    textColor: "{colors.ink}"
    typography: "{typography.mono}"
    rounded: "{rounded.none}"
    padding: "4px 8px"
  tooltip:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.sheet}"
    rounded: "{rounded.none}"
    padding: "6px 12px"
---

# Design System: Whetstone

## Overview

**Creative North Star: "The Standards Sheet"**

The dashboard draws the rulebook as a controlled engineering document. One sheet sits on a desk inside a 1px drawing frame with zone ticks in its margin: a title block across the top, then the purpose, then the principles, then numbered requirements grouped Hard rules then Soft rules, then the Changelog, a revision table that is the rulebook's Git history. Every value on the sheet is edited where it stands. It is not an admin console: there is no sidebar, no stat card and no data grid.

The sheet is ink on paper in light mode and bone on near-black in dark. Every division is a hairline. Corners are square, nothing casts a shadow, and nothing is a rounded card. Orange is the drawing's revision colour: it marks what differs from the last commit, the one primary action, and focus. Density is a drawing's density: the whole rulebook is visible as collapsed one-line rows, and opening a row draws a leader line into a framed callout.

Type does the work a drafting office would give it. Oswald caps set the title-block labels, requirement numbers, section heads and buttons. Vollkorn sets statements, rationale and running text. JetBrains Mono appears only for commands, paths, ids and short commit hashes.

**Key Characteristics:**
- A framed sheet (1px frame) on a darker desk, with zone ticks 1 to 6 and A to D from 1024px up.
- A title block of hairline-separated cells, each a small Oswald label over its value.
- Requirements numbered H1, H2… and S1, S2… per group, with three fixed stage cells (commit, push, CI).
- The delta triangle (Δ) as the only revision mark, orange when something is uncommitted.
- Radius 0 everywhere, hairline rules, no shadows.
- Motion that plots: lines draw first, text follows, expo out, none under reduced motion.

## Colors

Ink on a sheet on a desk, with one orange that means "this differs from the last commit" or "this is the action".

### Primary
- **Whetstone Orange** (`primary`): the primary button fill, the delta mark on drafted rows and the mission, the pending band's Δ, the active tab underline, the checked switch, the text caret, selection and the focus ring (`ring`). The same value in light and dark.
- **Pressed Orange** (`primary-hover`, `dark-primary-hover`): the primary button on hover; lighter in dark so it still lifts off the sheet.
- **Struck Orange** (`struck`, `dark-struck`): the deeper orange used for text-weight marks: the STONE half of the wordmark, struck-through old values (`.del`), the destructive outline button, action labels in diffs ("Added", "Changed"), "violates" in examples and the health line's problem icons. In dark it is the lighter `#ef7d4e` so it holds contrast.
- **Revision Wash** (`wash`, `dark-wash`): the pending band's background and the 1.2s flash behind a notice that just changed. It is the only tinted surface.

### Neutral
- **Desk** (`desk`, `dark-desk`): the page behind the sheet and the scrollbar track.
- **Sheet** (`sheet`, `dark-sheet`): the drawing surface: title-block cells, callouts, inputs, the Add rule panel.
- **Ink** (`ink`, `dark-ink`): text and the filled stage cell. In dark it is a warm bone, not white.
- **Ink 2** (`ink-2`, `dark-ink-2`): secondary prose: hints, summaries, rationale captions.
- **Ink 3** (`ink-3`, `dark-ink-3`): labels, unfilled stage cells, requirement numbers at rest, dates and hashes.
- **Rule** (`rule`, `dark-rule`): every hairline inside the sheet: row dividers, title-block gaps, chip and code borders. Also the shadcn `border` token.
- **Frame** (`frame`, `dark-frame`): the drawing frame, column-header underlines, the callout border, the leader line, strong chips and selected options. In dark it is a mid grey so the frame reads as a drawn line rather than a glare.
- **Card** (`card`, `dark-card`): the inset ground of commands, code examples and the commit-message preview, and the row hover (at 70%).
- **Muted** (`muted`, `dark-muted`, `dark-accent`): button and toggle hover grounds. `accent` and `secondary` equal `muted` in light.
- **Input** (`input`, `dark-input`): field borders and the unchecked switch.

### Named Rules
**The Orange Budget Rule.** Orange is spent only on the revision delta and pending marks, the one primary action per context, focus and selection, and struck-through or destructive text. Health problems borrow `struck` for their icon only. Everything else is ink.

**The One Fill Rule.** The primary button is the only solid orange fill on the sheet. A second solid orange shape in the same view is a mistake.

## Typography

**Display Font:** Oswald variable (200 to 700), with "Arial Narrow", sans-serif
**Body Font:** Vollkorn variable (400 to 900, roman and italic), with Georgia, serif
**Code Font:** JetBrains Mono variable (100 to 800, roman), with ui-monospace, "SF Mono", Menlo, Consolas, monospace. Weight 420 on the light sheet and 470 on the dark one; ligatures and contextual alternates off, slashed zero on, so `->` and `--` stay as typed.

**Character:** a condensed engineering capital beside a warm book serif. Oswald is always uppercase and tracked; Vollkorn is never uppercase. Numerals are lining and tabular throughout (`font-variant-numeric: lining-nums tabular-nums` on `body`). Fonts ship latin only, self-hosted, `font-display: swap`.

### Hierarchy
- **Purpose** (Vollkorn 500, 1.5rem, 1.875rem from 640px, 2.125rem from 1024px, line-height 1.25, max 44ch, balanced): the mission sentence. The largest text on the sheet.
- **Headline** (Oswald 600 caps, 1.375rem, line-height 1, tracking 0.04em): section heads ("Principles", "Hard rules (must)", "Soft rules (should)", "Retired", "Changelog"); the parenthetical strength in 1rem `ink-3`.
- **Title** (Oswald 500 caps, 1.125rem): panel heads such as "Add a rule" and setup steps. The setup step index is Oswald 600 at 2rem in `ink-3`.
- **Wordmark** (Oswald 600 caps, 1.0625rem, tracking 0.22em): WHET in ink, STONE in `struck`.
- **Statement** (Vollkorn 500, 1.0625rem, line-height 1.375): a rule's statement in its row; rationale in a callout is Vollkorn 400 at 1.0625rem, relaxed, max 68ch.
- **Body** (Vollkorn 400, 16px, line-height 1.55): running text. Secondary text is 0.9375rem; captions 0.875rem; the colophon 0.8125rem. Summaries and empty states are italic.
- **Number** (Oswald 600 caps, 0.8125rem): requirement numbers, `ink-3` at rest and ink when the row is open.
- **Button** (Oswald 500 caps, 0.8125rem, tracking 0.08em): every button.
- **Label** (Oswald 500 caps, 0.6875rem, tracking 0.1em, line-height 1.2, `ink-3`): title-block labels, column headers, field labels, zone ticks, stage cells (tracking 0.08em) and chips.
- **Code** (JetBrains Mono, 0.8125rem; 0.75rem for ids, digests and examples): commands, paths, branch, revision hashes. Inside running text it is 0.875em on a chip: `card` ground, 1px `rule` border, square, 0.05em by 0.32em padding, no wrap. Code that already sits in a frame (the copy command, path chips, title-block cells, hashes) is `bare`. Server text marks code with backticks and the page sets it as a chip.

### Named Rules
**The Caps Are Oswald Rule.** Anything set in capitals is Oswald and tracked. Vollkorn is never set in caps, and Oswald never sets a sentence.

**The Mono Is Literal Rule.** Mono is only for text a terminal or Git would accept: commands, paths, ids, hashes, the rulebook file name. Any such text in a sentence is wrapped in `<code>`.

## Layout

The page is a desk with one sheet on it. The sheet is centred at a maximum of 84rem. From 1024px the desk has a 32px margin and the sheet a 24px margin, which holds the zone ticks: six columns numbered 1 to 6 along the top and bottom and four rows lettered A to D down the sides, each tick a label cell separated by a `rule` hairline. The inner frame is a 1px `frame` border. Below 1024px the ticks and inner border go and the sheet fills the viewport, still bounded by the outer frame edges.

Inside the sheet, from the top: the title block, the read-only note, the pending band, the health line, then the body, then the colophon. Bands run full width with 16px side padding (24px from 640px). The body has 16px, 32px from 640px, and 40px from 1024px of side padding.

The body is a two-column drawing from 1024px: a 10rem margin column holding each section's label or head, and the content column, 32px apart. Below 1024px the label sits above its content. Sections are 48px apart; the two groups 40px; the rulebook and the Changelog 64px. Principles, both rule groups, Retired and the Changelog fold: the head in the margin column is a button with a chevron (rotated -90° when folded), the content collapses to one `rule`-bordered line naming its count with a Show label, and the choice is kept per browser (`wh-collapsed`). The mission never folds.

The title block is a grid of cells with 1px `rule` gaps: 2 columns at phone width, 4 from 640px, and one row of seven from 1024px (logo, project, branch, revision, contents, last change, mode and controls).

A requirement row is a grid from 1024px: number (2.5rem), statement (fluid), checked by (5.5rem), runs at (11.5rem), when broken (6rem), chevron (1.25rem), with 12px gaps; Oswald column headers sit over a `frame` underline. Below 1024px the row is number, statement and chevron, with the chips and stage cells wrapping under the statement. The revision table is rev (6rem), date (7.5rem), by (9rem), description from 640px; below that it is rev and description with the date and author underneath.

Breakpoints are Tailwind's: 640px (`sm`) and 1024px (`lg`). The page is checked at 320, 390, 768 and 1024px.

The page's HTML, CSS and JavaScript stay under 600 KiB and its fonts under 200 KiB, all served by `wh` itself.

## Elevation & Depth

There is no elevation. The system uses no `box-shadow` and no blur. Depth is the desk, the sheet and the frame: the desk is darker than the sheet, the frame is a drawn 1px line, and inset content (commands, code, the commit preview) sits on `card`. Something that opens, such as a requirement's detail, is framed with a 1px `frame` border and joined to its source by a leader line, never lifted.

### Named Rules
**The Drawn Not Lifted Rule.** A surface that needs separating gets a hairline or the frame, never a shadow. A shadow on the sheet is a defect.

## Shapes

Every corner is square: `--radius` and Tailwind's `radius-sm` to `radius-xl` are all 0, and the shadcn primitives carry `rounded-none`. Borders are 1px: `rule` inside the sheet, `frame` for the frame, the callout, strong chips and the selected option. The only 2px strokes are the focus outline, the active tab underline and the insertion underline. The recurring geometries are the square stage cell (0.55rem, 1px border, filled when the rule runs there), the delta triangle (a 16 by 14 outline with miter joins, 1.4px stroke, 1px when it carries a number), and the leader line (an L from the requirement number down and across into the callout, 1px `frame`).

The Whetstone stone is the only illustrated mark: an isometric stone in three oranges (`stone-top`, `stone-left`, `stone-right`) on an ink base, at 32px in the title block.

## Components

Restyled shadcn primitives with Radix behaviour: square, hairline, Oswald on anything that is a control.

### Buttons
- **Shape:** square (0px), Oswald 500 caps at 0.8125rem, tracking 0.08em; 36px default, 32px small.
- **Primary:** `primary` fill with ink text (`primary-foreground`); one per context: Accept, Add rule, Commit to the rulebook, Save draft.
- **Hover / Focus:** colour transitions only, 150ms; primary darkens to `primary-hover`. Focus is a 2px `ring` outline offset 2px.
- **Outline:** 1px `frame` border on the sheet, hover `muted`. Used for Withdraw, Make it a soft rule, Make it a hard rule, Show all.
- **Ghost:** no border, hover `muted`; Review, Cancel, Close, Edit mission, the TSV download.
- **Destructive:** a 1px `struck` outline with `struck` text and a `wash` hover, never a filled red: Retire this rule, Discard these changes.

### Chips
- **Style:** Oswald caps at 0.6875rem, 20px tall, 6px padding, transparent ground. Strong chips (the check kind, "Fails gate") have a `frame` border and ink text; quiet chips (the sweep class: Discuss, Ask first, Fix PR) have a `rule` border and `ink-2` text.
- **Badge:** the mode badge (Editing or Read-only) is a strong chip with a tooltip.

### Inputs / Fields
- **Style:** 1px `input` border, square, sheet ground (`input` at 30% in dark), Vollkorn; mono at 0.8125rem for commands, queries and globs.
- **Focus:** the border turns `ring` and a 2px `ring` outline sits flush (offset 0).
- **Options:** strength options are bordered boxes (`rule`, `frame` when checked); the check kind is a segmented toggle group of Oswald caps whose selected item inverts to ink fill with sheet text.
- **Disabled:** 50% opacity.

### Navigation
There is no navigation chrome. The title block's last cell holds the mode badge, the Ids switch (a square 32 by 18px track, `primary` when on) and a three-way theme toggle (system, light, dark) of 28px square icon items. A skip link goes to the rulebook.

### Title Block
A row of cells on a `rule` ground with 1px gaps. Each cell is a label over a 0.9375rem value, truncated with a title attribute; project is Oswald 500, branch and revision are mono. On load the cells fade up 4px in a 40ms stagger after the frame is plotted.

### Requirement Row
Collapsed, one line: number, Δ if drafted, statement, the check-kind chip, the three stage cells, the sweep chip or "Fails gate", and a chevron; hover is `card` at 70%. Opened, a 1px leader line draws from the number (240ms, quad out) and the framed callout unrolls downward with a clip-path (320ms, expo out, 120ms after the line). The callout lists Why, Checked by (with a copyable command), Applies to, Runs at, When broken, Examples, Source, History and, with Ids on, the id and digest, each as an Oswald label in an 8.5rem column beside its value.

### Inline Editing
Every value on the sheet is edited where it stands, in its own type: the mission, a principle, a rule's statement, Why, command, paths and language are `contenteditable` (plaintext only) on the element that shows them. A dotted `ink-3` underline appears on hover. Focused, nothing frames it: it is writing on the page, with the orange caret and the same faint dotted underline under the text so focus stays visible. Enter (or leaving the field) writes a draft, Shift+Enter breaks a line where a value allows it, Escape restores. Choices (check type, sweep class, priority, strength) are toggle groups in place, and each change writes a draft. A check that cannot run here is refused in an inline alert with "Save it anyway, unvalidated". New things (a rule from Add rule, the first mission, a retirement reason) still use fields, because there is no value to edit yet.

### Redline
A drafted change is redlined: a 2px `primary` underline 5px under each line of the changed text (`text-decoration`, so it follows the words, not the box). It comes up over 420ms (colour from transparent, offset from 9px to 5px, expo out) when the draft lands, and leaves when the change is accepted or withdrawn. Fields come from the pending diff, so only what changed is marked.

### Stage Cells
Three fixed cells, always in the order commit, push, CI, each a 0.55rem square before an Oswald caps label. Filled ink with an ink label where the rule runs; an empty outline with an `ink-3` label where it does not. Cells never move or disappear, so columns of rows read like a timetable.

### Delta Mark and Numbering
The Δ is the revision mark. Unnumbered and orange, it marks a drafted rule, principle or mission and leads the pending band. Numbered, it is the revision number in the revision table, `ink-2` at rest and orange only on the revision just accepted, which is stamped in (scale 1.35 to 1, rotate -6° to 0, 480ms).

Requirement numbers are per-group ordinals for the revision on screen: H1 to Hn for hard rules, S1 to Sn for soft rules. They are not ids and shift when rules are added or retired; the rule id is the identity and appears with the Ids switch. Revisions are numbered oldest first, so the newest row carries the highest number.

### Pending Band
Shown only when the rulebook file differs from its last commit: a `wash` band under a `frame` rule with a Δ, the count in Oswald caps beside the mono file name, an italic summary, and Review, Withdraw and Accept. Review opens the field diff beside the commit message accepting would write.

### Diff
Before and after per field in a table: old values struck through in `struck` on `ink-3` text, new values underlined 2px in `primary` with a 3px offset. The action label (Added, Changed, Retired) is a `struck` label.

### Command
A copyable command: mono text on `card` inside a 1px `rule` border, with a copy button behind a `rule` divider that swaps to a check for 1.4s.

### Motion
Anime.js, one grammar: lines draw, then text follows, ease `outExpo`. The frame plots edge by edge (scale 0 to 1, 520ms, the bottom and right edges 180ms later), title-block cells fade up from 160ms, requirement rows arrive left to right in rank order (360ms, 16ms stagger from 240ms), and the leader and stamp behave as above. The gate pass is the focal moment: opening a rule runs a 1px `primary` line under its stage cells, commit to push to CI (520ms, quad in-out), filling each gate the rule runs at as the line reaches it, then fades the line (360ms). It says where the rule bites. A changed notice flashes `wash` to transparent over 1.2s. Every element is visible before its animation runs, and under `prefers-reduced-motion` no animation runs, CSS animations and transitions drop to 0.01ms, and the stage cells and redline simply appear.

## Do's and Don'ts

### Do:
- **Do** keep every radius at 0 and every rule at 1px; reserve 2px for focus, the active tab and the insertion underline.
- **Do** put new controls and labels in Oswald caps at the label or button size, and prose in Vollkorn.
- **Do** spend orange only within the Orange Budget Rule, and keep one solid orange fill per view.
- **Do** mark anything uncommitted with the Δ, redline the changed text, and number requirements per group (H1, S1) from the rules on screen.
- **Do** edit values where they stand; never swap a value for a form to change it.
- **Do** show where a rule runs with the three fixed stage cells, filled or empty, never by hiding a cell.
- **Do** separate surfaces with `rule` hairlines and the `frame`, and inset literal text on `card`.
- **Do** animate by drawing a line first and letting text follow, 240 to 520ms, `outExpo`, and skip it under reduced motion.

### Don't:
- **Don't** add a sidebar, stat cards, a health score or a data grid; the page is one sheet.
- **Don't** use `box-shadow`, blur or rounded cards.
- **Don't** fill a destructive button; it is a `struck` outline.
- **Don't** set Vollkorn in caps, or use mono for anything a terminal would not accept.
- **Don't** treat requirement numbers as identities; link and refer to rules by id.
- **Don't** delete a retired rule from view; it stays collapsed at the end of the rulebook with its reason, its id struck through.

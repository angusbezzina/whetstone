---
name: Whetstone
description: A quiet engineering journal where the project's records are entries on ruled paper, in light or dark, and colour is spent only on state.
colors:
  paper: "#fafaf8"
  well: "#f0f0ec"
  rule: "#e2e1dc"
  rule-strong: "#bfbeb8"
  ink: "#1e1d1b"
  ink-soft: "#5e5c57"
  ink-faint: "#6a6963"
  accent: "#2b5797"
  selection: "#d5e1f4"
  pass: "#2e7a3f"
  warn: "#8a5a00"
  fail: "#b3261e"
  dark-paper: "#181817"
  dark-well: "#212120"
  dark-rule: "#2d2d2b"
  dark-rule-strong: "#484742"
  dark-ink: "#e8e6e1"
  dark-ink-soft: "#a9a69f"
  dark-ink-faint: "#96938c"
  dark-accent: "#8db2ea"
  dark-selection: "#2b3f5f"
  dark-pass: "#74c682"
  dark-warn: "#d9a94f"
  dark-fail: "#f0837b"
typography:
  headline:
    fontFamily: "ui-serif, 'New York', 'Iowan Old Style', Georgia, 'Times New Roman', serif"
    fontSize: "27px"
    fontWeight: 400
    lineHeight: 1.25
    letterSpacing: "-0.005em"
  title:
    fontFamily: "ui-serif, 'New York', 'Iowan Old Style', Georgia, 'Times New Roman', serif"
    fontSize: "19px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "-0.005em"
  wordmark:
    fontFamily: "ui-serif, 'New York', 'Iowan Old Style', Georgia, 'Times New Roman', serif"
    fontSize: "18px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "-0.005em"
  day:
    fontFamily: "ui-serif, 'New York', 'Iowan Old Style', Georgia, 'Times New Roman', serif"
    fontSize: "17px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "-0.005em"
  item:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "17px"
    fontWeight: 500
    lineHeight: 1.4
    letterSpacing: "-0.005em"
  body:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  body-name:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "15px"
    fontWeight: 500
    lineHeight: 1.6
    letterSpacing: "normal"
  nav:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "14.5px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  control:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.3
    letterSpacing: "normal"
  summary:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  meta:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "13.5px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  label:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  figure:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  code:
    fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  brief:
    fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.55
    letterSpacing: "normal"
  note:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  mono-small:
    fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  headline-narrow:
    fontFamily: "ui-serif, 'New York', 'Iowan Old Style', Georgia, 'Times New Roman', serif"
    fontSize: "23px"
    fontWeight: 400
    lineHeight: 1.25
    letterSpacing: "-0.005em"
rounded:
  r: "4px"
  well: "6px"
  inner: "3px"
  dot: "50%"
spacing:
  hair: "1px"
  xs: "4px"
  sm: "8px"
  row-y: "0.7rem"
  md: "16px"
  well: "1rem 1.1rem"
  lg: "24px"
  section: "2.25rem"
  page-x: "1.5rem"
  page-max: "48rem"
components:
  button:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.control}"
    rounded: "{rounded.r}"
    padding: "0.4rem 0.85rem"
  button-hover:
    backgroundColor: "{colors.well}"
    textColor: "{colors.ink}"
  button-primary:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.paper}"
    typography: "{typography.control}"
    rounded: "{rounded.r}"
    padding: "0.4rem 0.85rem"
  button-primary-hover:
    backgroundColor: "{colors.ink-soft}"
    textColor: "{colors.paper}"
  button-quiet:
    backgroundColor: "transparent"
    textColor: "{colors.ink-soft}"
    typography: "{typography.control}"
    rounded: "{rounded.r}"
    padding: "0.4rem 0.85rem"
  button-quiet-hover:
    backgroundColor: "{colors.well}"
    textColor: "{colors.ink}"
  button-danger:
    backgroundColor: "transparent"
    textColor: "{colors.fail}"
    typography: "{typography.control}"
    rounded: "{rounded.r}"
    padding: "0.4rem 0.85rem"
  input:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    rounded: "{rounded.r}"
    padding: "0.45rem 0.6rem"
  section-heading:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    typography: "{typography.title}"
    padding: "0 0 0.4rem"
  row:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    typography: "{typography.body-name}"
    padding: "0.7rem 0"
  well:
    backgroundColor: "{colors.well}"
    textColor: "{colors.ink}"
    rounded: "{rounded.well}"
    padding: "1rem 1.1rem"
  tab:
    backgroundColor: "transparent"
    textColor: "{colors.ink-soft}"
    typography: "{typography.nav}"
    padding: "0.6rem 0 0.65rem"
  tab-selected:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
  tab-disabled:
    backgroundColor: "transparent"
    textColor: "{colors.ink-faint}"
  theme-toggle:
    backgroundColor: "transparent"
    textColor: "{colors.ink-faint}"
    rounded: "{rounded.inner}"
    padding: "0.1rem"
  state-pass:
    textColor: "{colors.pass}"
    typography: "{typography.figure}"
  state-warn:
    textColor: "{colors.warn}"
    typography: "{typography.figure}"
  state-fail:
    textColor: "{colors.fail}"
    typography: "{typography.figure}"
  state-muted:
    textColor: "{colors.ink-soft}"
    typography: "{typography.figure}"
  state-draft:
    textColor: "{colors.ink-soft}"
    typography: "{typography.figure}"
  command-chip:
    backgroundColor: "{colors.well}"
    textColor: "{colors.ink}"
    typography: "{typography.code}"
    rounded: "{rounded.r}"
    padding: "0.12rem 0.45rem"
  brief-block:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    typography: "{typography.brief}"
    rounded: "{rounded.r}"
    padding: "0.7rem 0.8rem"
---

# Design System: Whetstone

<!-- Recorded from planning/direction-demo/index.html on 2026-09-10 after the owner pinned the Notebook world, replacing Graphite Console. The implementation target is assets/dashboard/. Token names below are the CSS custom properties declared on :root in the artifact (--paper, --well, --rule, --rule-strong, --ink, --ink-soft, --ink-faint, --accent, --selection, --pass, --warn, --fail, --serif, --sans, --mono, --r, --ease, --t). The light values are the canonical entries; the dark-* keys are the same tokens as redefined under prefers-color-scheme: dark (guarded :root:not([data-theme="light"])) and again under :root[data-theme="dark"]. -->

## Overview

**Creative North Star: "The Engineering Journal"**

Whetstone's dashboard is a journal the project keeps about itself, not a console and not a report. One page column, 48rem wide, sits centred with generous margins on plain paper; every section is a serif heading on a rule, and every record is an entry on a ruled line beneath it. Nothing is boxed, nothing floats, nothing is tinted. The eye moves down the page the way it moves down a notebook: heading, rule, entries, more space, next heading. Density is comfortable rather than packed: 15px body on a 1.6 rhythm, entries about 3.2rem tall, sections 2.25rem apart.

The page reads in either light: paper and ink by day, a warm near-black ground with off-white ink at night, chosen by the operating system or pinned by a three-way toggle in the header. Both renditions share every rule and every ratio; only the tokens swap, and every text-on-ground pair in both modes clears 4.5:1.

Colour is spent only on state. The primary action is ink, not a brand colour; the one accent, a fountain-pen blue, is reserved for the focus ring, caret and text selection. Green is a fresh real pass or an accepted record; ochre is honest doubt (unknown, stale, not run, not observed); red is a failure. Drafts, superseded records and not-yet-available tabs are marked by dashed rules and hollow or dashed rings, never by a colour of their own. A page with nothing to report is almost entirely ink on paper, and that calm is the point.

The world refuses the SaaS dashboard (sidebar of icons, KPI tiles, card grids, shadows, health scores) and the graphite console it replaces (panels, seams, tracked uppercase labels, mono-as-costume). It also refuses the cream-paper-and-display-serif template: the paper is a near-neutral off-white, the serif is the operating system's book face at text sizes, and there is no warm accent.

**Key Characteristics:**
- One calm column of paper, 48rem max, 1.5rem side margins; no panels, cards or shadows anywhere.
- Ruled hairlines carry all structure: 1px rules between entries, a slightly stronger rule under each section heading and column header.
- Serif headings (view title, section title, mission, day) at 400 weight; sans for everything else; mono only for commands, references, hashes and exact records.
- Colour only on state: green pass or accepted, ochre unknown or stale, red fail; ink for the primary action; blue only for focus, caret and selection.
- Light and dark from one token set; dark is a redefinition, never a second design.
- Drafts are dashed: a dashed rule under a draft entry, a dashed ring on a draft state, a dashed dot after a gated tab.
- Motion is a single 180ms ease-out along the vertical axis (grid-template-rows reveals, chevron rotation, colour), removed under prefers-reduced-motion.

## Colors

A paper-and-ink neutral ramp with three state voices and one reserved accent, in a light rendition and a dark one.

### Primary
- **Ink** (`--ink`, #1e1d1b light / #e8e6e1 dark): headings, names, values, the selected tab and its underline, the primary button fill, the "after" cell's border. The strongest thing on the page is ink, not colour.
- **Paper** (`--paper`, #fafaf8 light / #181817 dark): the page, inputs, the brief block inside a well, and the text on a primary button.

### Secondary
- **Accent** (`--accent`, #2b5797 light / #8db2ea dark): the 2px focus outline, the input focus border and 1px ring, the caret. Never a button, never a link colour, never decoration. **Selection** (`--selection`, #d5e1f4 light / #2b3f5f dark) is the text-selection ground with ink text.

### Tertiary
- **Pass** (`--pass`, #2e7a3f light / #74c682 dark): only a fresh real pass on the current revision, an accepted changelog record, the pass tally, the single "written" effect in a draft review.
- **Warn** (`--warn`, #8a5a00 light / #d9a94f dark): unknown, stale, not run, not observed. Ochre rather than yellow so it clears 4.5:1 on paper. The tally labels it "unknown", never "warnings".
- **Fail** (`--fail`, #b3261e light / #f0837b dark): a failing gate, an off-target metric, a failure location in a checks detail, the danger button.

### Neutral
- **Well** (`--well`, #f0f0ec light / #212120 dark): the recessed ground of an editor form, a draft review, a checks detail, a command chip, an exact-record block, and hovered controls. One step back from paper.
- **Rule** (`--rule`, #e2e1dc light / #2d2d2b dark): the 1px hairline between entries, under the tab row, under the demo strip, around the brief block, and above the colophon.
- **Rule Strong** (`--rule-strong`, #bfbeb8 light / #484742 dark): the rule under a section heading, a day heading or a column header; borders on inputs, secondary buttons and diff cells; scrollbar thumbs.
- **Ink Soft** (`--ink-soft`, #5e5c57 light / #a9a69f dark): descriptions, meta lines, counts, versions, unselected tabs, quiet buttons, form labels, muted and draft states, the primary button on hover.
- **Ink Faint** (`--ink-faint`, #6a6963 light / #96938c dark): the quietest tier, still at or above 4.5:1 on paper and well: placeholders, keys in detail lines, section descriptors beside a heading, gated tabs, superseded records, notes under a confirmation, the colophon, the demo strip.

### Named Rules
**The State-Only Colour Rule.** Green, ochre and red appear only to report a state. Blue appears only on focus, caret and selection. Nothing else on the page is coloured: no tinted categories, no coloured icons, no brand colour, no chart fills. If an element is coloured, a reader must be able to say what state it reports.

**The Never-Green Rule.** Unknown, stale, not run, not observed, not installed and draft are ochre or ink-soft. Green is reserved for a fresh pass on the current revision or an accepted record. Read-only inspection is never green.

**The Ink Action Rule.** Each view carries one primary action, filled in ink with paper text. Every other button is outlined, quiet or red. Emphasis comes from ink, weight and space, never from a brand colour.

**The Two-Light Rule.** Dark mode redefines tokens only. No selector, size, weight, radius or rule changes between modes, and every text pair in both modes is verified at or above 4.5:1.

## Typography

**Display Font:** none.
**Headline Font:** ui-serif (New York on Apple platforms; Iowan Old Style, Georgia, Times New Roman fallbacks)
**Body Font:** ui-sans-serif, system-ui (with -apple-system, Segoe UI, Roboto, Helvetica Neue, Arial fallbacks)
**Code Font:** ui-monospace (with SF Mono, Menlo, Consolas, Liberation Mono fallbacks)

**Character:** Set like a well-kept notebook. The operating system's book serif carries the headings at text sizes and regular weight, so it reads as a heading in a journal rather than a display face on a website. The system sans carries every entry, control and figure; the page is set with tabular numerals throughout so times, versions and counts align without a monospace costume. Mono is reserved for things that are literally code: commands, references, source paths, hashes and exact records. All faces come from the operating system because the product forbids third-party requests and its startup budget is 24 KiB; that constraint is pinned.

### Hierarchy
- **Headline** (serif, 400, 27px, 1.25, -0.005em): the view title (Foundations, Checks, Changelog), the mission line on the dashboard, the onboarding heading. Balanced wrapping, max 30ch. One per view. 23px under 390px.
- **Title** (serif, 400, 19px, 1.4): every section heading (Needs attention, Key metrics, Gates, each of the five Foundations stages) and the mission record in Foundations. The heading sits on a strong rule with its count at right in figure type.
- **Wordmark** (serif, 400, 18px): "Whetstone" in the header, beside the project name in sans meta.
- **Day** (serif, 400, 17px): a changelog day heading, with the day's count at right.
- **Item** (sans, 500, 17px, -0.005em): the primary Needs-attention title.
- **Body** (sans, 400, 15px, 1.6): descriptions, prose, definition values, form fields. Descriptions cap at 58–64ch.
- **Body Name** (sans, 500, 15px): entry names, gate names, changelog titles, the Latest change value, list item names in onboarding.
- **Nav** (sans, 14.5px): the four view tabs; selected is ink at 500 with a 1px ink underline.
- **Control** (sans, 14px, 1.3): every button.
- **Summary** (sans, 14px): a changelog summary, desired outcomes, diff cell content.
- **Meta** (sans, 13.5px, ink-soft): the line beneath a name: mechanism and reference, target and window, actor and next step, the check line, the advisory note.
- **Label / Figure** (sans, 13px): form labels, column headers, counts, versions, times, state labels, the header state cluster, the stages index, summaries of disclosures. Figures use tabular numerals; states take their state colour.
- **Code** (mono, 12.5px): inline commands, references and paths; the command chip; the scope input.
- **Brief** (mono, 12.5px, 1.55): the repair brief and exact-record blocks, pre-wrapped.
- **Note** (sans, 12.5px, ink-faint): the demo strip, the colophon, notes beneath a confirmation.

### Named Rules
**The Heading-on-a-Rule Rule.** A section's heading is a serif title sitting on a strong rule, with its count at right and an optional sans descriptor in ink-faint beside it. Nothing sits above a heading as a kicker, eyebrow, overline or category tag. Section numbers appear only in the onboarding list, where the order is the content.

**The Literal-Mono Rule.** Monospace is used only for text a terminal would accept or emit: commands, references, source paths, hashes, exact records. Times, versions, counts and states are set in the sans with tabular numerals.

## Layout

The page is a single centred column: `.app` at max-width 48rem with 1.5rem side padding (1.1rem under 768px, 1rem under 390px) and 4rem of bottom room. Above it, a demo strip that is not the product recedes to 12.5px ink-faint under a hairline. The header is two lines: brand row (wordmark, project, agreement state, draft count, theme toggle) and the tab row, a sticky hairline of four text tabs that stays at the top while the page scrolls. The main column begins 2rem beneath the tabs and ends in a colophon above a hairline (where the dashboard is served, and that it shows the same records as `wh dash --json`).

Every view stacks its sections vertically at 2.25rem intervals. A section is a heading on a strong rule followed by entries on hairlines. An entry is a grid whose last columns are `auto` so state, version and Edit sit flush right on the first line, with meta beneath the name. The dashboard stacks Needs attention, Key metrics and Gates, then a single Latest change line. Foundations opens with a one-line stages index (name and count, separated by small stroke arrows) and runs the five stages in order. Checks carries a check line (last run, scope, tally; the run form on its own line beneath), then a three-column board (gate with mechanism beneath, last run, result, chevron). Changelog groups entries under serif day headings with a 4.5rem time column.

Breakpoints, in order:
- **768px**: side padding 1.1rem; tab gap tightens; the board hides its last-run column and lets the mechanism line wrap; details, diffs and two-column forms go to one column; entries reflow with Edit at top right and version beneath the name; the Latest change line stacks its time.
- **390px**: side padding 1rem; headlines drop to 23px; the tab row wraps onto two lines instead of scrolling; the agreement detail leaves the header; metric and gate lines stack their state under the name; changelog entries put time and status on one line above the title.

The product commitment is 320, 390, 768 and 1024px; the artifact hits 390, 768 and 1024 explicitly and 320 by the same rules, with no horizontal overflow at any of them or at 1440.

**The One-Axis Motion Rule.** Reveals open along the vertical axis by animating `grid-template-rows` from 0fr to 1fr over 180ms with cubic-bezier(.2,.7,.2,1). Chevrons rotate 90° on the same clock; colour and border transitions share it. Nothing slides sideways, scales or fades in. `prefers-reduced-motion: reduce` removes every transition and animation. The only continuous motion is a 0.8s spinner inside a busy primary button and a 1s pulse on the state dot of a running gate.

## Elevation & Depth

This system has no shadows and no raised surfaces. Depth is two grounds: paper, and one recessed well one step darker (lighter in dark mode) for anything that is being worked on or inspected: an editor form, a draft review, a checks detail, an exact-record block, a command chip. Inside a well, inputs and the brief block return to paper so the thing you type or copy reads as the page again. The only `box-shadow` in the build is the 1px accent ring on a focused input, which is a focus indicator, not elevation.

### Named Rules
**The Ruled-Line Rule.** Structure is drawn with 1px rules: hairline between entries, strong under headings and column headers. Never a box, border-all-round panel, shadow, glow, blur or gradient.

**The Dashed Rule.** Anything not in force is dashed: a draft entry's rule, a draft or not-run state's ring, the dot after a gated tab, the demo strip's separation is not dashed because it is not a state.

## Shapes

One 4px radius on buttons, inputs, chips, diff cells, disclosure markers and the focus outline; 6px on the recessed wells so they read as a soft inset in the page; 3px inside the theme toggle. Circles only for the 6px state dot, the dashed ring on a draft state, and the busy spinner. Corners never go to 0 and never go pill-shaped. Borders are always 1px: solid for inputs, secondary buttons and diff cells; the "after" cell's border is ink. Disclosure markers, chevrons, arrows, copy, play, plus and the three theme icons are all inline SVG at stroke 1.5 with round caps; no unicode glyphs or emoji stand in for icons, and no icon appears without its label except the three theme buttons, which carry aria-labels and titles.

## Components

### Buttons
One vocabulary, four voices, all inline-flex with a 0.4rem icon gap and 14px inline SVG.
- **Shape:** 4px radius, 1px border, 0.4rem × 0.85rem padding, 14px, line-height 1.3, nowrap.
- **Secondary (default `.btn`):** transparent with a rule-strong border and ink text; hover draws the border in ink on a well ground; active on rule. Used for Cancel, Back, Copy, Copy brief.
- **Primary (`.btn.primary`):** ink fill, paper text, weight 500; hover to ink-soft. Exactly one per view. A busy primary hides its text and shows a 14px paper spinner.
- **Quiet (`.btn.quiet`):** transparent, no visible border, ink-soft text; hover to ink on well. Used for Edit, Add value/metric/guideline/gate, secondary attention items, See rules, Clear, Reset sample data.
- **Danger (`.btn.danger`):** transparent with fail text; hover draws a fail border. Reserved for withdraw and remove.
- **Focus:** 2px accent outline at 2px offset, 4px radius, on every focusable element via `:focus-visible`.
- **Disabled:** 50% opacity, not-allowed cursor.

### State Labels
- **Style:** 13px sans preceded by a 6px filled circle in `currentColor`; inline-flex with 0.45rem gap; nowrap; flush right in its entry.
- **State:** `pass` green, `fail` red, `warn` ochre, `muted` ink-soft with a hollow ring, `draft` ink-soft with a dashed ring. The label is the honest word: "pass", "pass · stale", "fail · 2", "not run", "draft · not run", "not observed", "8% · off target", "up to date". A running row pulses its dot.

### Sections
- **Heading:** serif title on a 1px rule-strong, 0.4rem beneath the text; count at right in 13px ink-soft; optional descriptor in 13px ink-faint beside the title.
- **Entries:** 0.7rem vertical padding, 1px hairline beneath each; name at 500, meta beneath at 13.5px ink-soft, state / version / Edit flush right spanning both lines.
- **Empty:** 0.9rem of ink-soft prose on a hairline stating what is missing and why it is not success.
- **Latest change:** a full-width quiet button set as a ruled line 1.5rem below the last section: key in ink-soft, value at 500, time in 13px ink-soft; hover underlines the value.

### Inputs / Fields
- **Style:** paper ground, 1px rule-strong border, 4px radius, 0.45rem × 0.6rem padding, inherits body type, full width, accent caret, ink-faint placeholder. Textareas start at 4.4rem and resize vertically. A label is 13px ink-soft stacked above its field with 0.3rem gap.
- **Hover:** border steps to ink-faint.
- **Focus:** border goes accent plus a 1px accent ring; the outline is suppressed in favour of the ring.
- **Scope input in the check line:** 9rem wide, mono 12.5px.

### Navigation
- **Tab row:** four text tabs in a sticky row on a hairline, 1.5rem apart (1.1rem under 768px, wrapping under 390px); each tab is a transparent button, 14.5px ink-soft, 0.6rem vertical padding, with a transparent 1px bottom border overlapping the row's rule. Hover: ink. Selected (`aria-selected=true`): ink, weight 500, 1px ink underline. Gated (`aria-disabled=true`): ink-faint with a small dashed ring after the label and a title explaining when it becomes available. The row is a `tablist` with arrow, Home and End navigation.
- **Header:** brand row with the 15px stroke mark, serif wordmark, project name in sans meta; at right the agreement state, draft count and a three-button theme toggle (system / light / dark) with `aria-pressed`, remembering the choice in localStorage inside try/catch and applying it before first paint.
- **Stages index (Foundations):** one line of 13.5px ink-soft links with counts, separated by 12px stroke arrows; hover underlines.

### Checks Board (signature)
A ruled table with a three-column grid plus a chevron (gate / last run / result / 1.6rem) so results update line by line in place. The column header is 13px ink-faint on a hairline. Each line: gate name at 500 with a faint strength tag, mechanism · reference · scope beneath in 13px ink-soft (ellipsised on desktop, wrapped under 768px), last run in 13px, the state label, and a chevron that rotates 90° when expanded. Beneath a line a well opens along the vertical axis (grid-template-rows 0fr → 1fr, 180ms), two columns at 1.1rem gap: Failures (or Result / Status) as a 13px ink-soft heading, each failure as its path in red mono over its description, the exact recheck command as a command chip with a Copy button; and, for a failing gate, the repair brief as a paper block with ink-faint keys, a note stating it starts no agent, and a Copy brief button. Above the board a check line carries last run, scope and a pass / fail / unknown tally in state colours, with the scope input and the one primary Run checks action on their own line beneath.

### Record Entry and Draft Editor (signature)
A Foundations entry is a four-column grid (content / state / version / Edit): content at 500 capped at 64ch (the mission in serif title type with its outcomes as a dashed list beneath), a meta line, the version in 13px with the number in ink, and a quiet Edit button. A draft entry sits on a dashed rule and sets its version in ink-soft with "draft" in italic. Edit opens a well along the vertical axis containing a stacked form: a 13px base line (base version in ink, "draft stays private" at right), labelled fields, and a right-aligned Cancel / Review draft pair. Review replaces the form with a before / after diff in two 4px paper cells (after carries an ink border and label), a why / expected line, an effects line where only "written" is green and every "no" is ink-soft, and a confirmation row isolated above its own hairline: an ink-faint note on the left stating what will not happen and Back / Record local draft on the right.

### Changelog Entry
Grouped under a serif day heading on a strong rule with the day's count at right. Each entry: time in 13px at 4.5rem, title at 500 with the version in 13px ink-soft, a 14px ink-soft summary, a 13px ink-faint meta line, and a state label at right where accepted is green and superseded is ink-faint with a hollow ring. Exact records sit behind a `details` disclosure whose summary carries a rotating stroke chevron, rendered as a diff or an ink-soft mono block on well. Entries after the as-of time sit on a dashed rule, set all their text in ink-faint, hide their disclosure and say so in their meta line; they are never dimmed by opacity, so they stay legible.

### Onboarding Page
The pre-init dashboard is one section without a heading rule: a serif headline (This project has no agreement yet.), ink-soft prose at 58ch, a numbered ruled list of the eight decisions (numbers in 13px ink-faint, names at 500, descriptions in ink-soft), the single primary action beside an ink-faint "or from the terminal" and the `wh init` command chip, and an ink-faint line stating nothing is shared or installed.

## Do's and Don'ts

### Do:
- **Do** set every view as one centred column of paper (48rem, 1.5rem margins) and draw all structure with 1px rules: hairline between entries, strong under headings.
- **Do** define the complete light palette on `:root`, redefine only the tokens under `prefers-color-scheme: dark` guarded as `:root:not([data-theme="light"])`, and again under `:root[data-theme="dark"]`; verify every text pair at or above 4.5:1 in both.
- **Do** spend colour only on state: green for a fresh pass or accepted record, ochre for unknown, stale, not run or not observed, red for fail or off target; ink for the one primary action; blue only for focus, caret and selection.
- **Do** set headings in the system serif at 400 weight and text sizes (27 / 19 / 17px), everything else in the system sans, and only literal commands, paths, hashes and records in mono.
- **Do** mark what is not in force with dashes: a dashed rule under a draft entry, a dashed ring on a draft state, a dashed dot after a gated tab.
- **Do** open reveals along the vertical axis with `grid-template-rows` over 180ms and remove all motion under `prefers-reduced-motion`.
- **Do** isolate consequential confirmations above their own hairline with an ink-faint note stating what will not happen and the primary action at right.
- **Do** keep the tab row a labelled `tablist` that stays sticky, tightens under 768px and wraps under 390px, and hold up at 320, 390, 768, 1024 and 1440px with no horizontal overflow.
- **Do** draw every icon as inline SVG at stroke 1.5 beside its label, or with an aria-label where the control is icon-only (the theme toggle).

### Don't:
- **Don't** box anything: no panels, cards, borders-all-round containers, shadows, glows, gradients or blurs; the only `box-shadow` is the 1px accent focus ring on an input.
- **Don't** place a kicker, eyebrow, overline, tracked uppercase label or category tag above a heading; the serif title on its rule is the only heading.
- **Don't** colour anything that is not a state: no brand colour on buttons, no tinted categories, no coloured icons, no chart fills, no warm accent.
- **Don't** show green for a stale pass, an unknown result, a draft, an inspection or anything not observed on the current revision.
- **Don't** add a blended health score, progress ring, KPI tile, sparkline, gamified streak or fleet-style status wall.
- **Don't** load a web font, an icon font, an image or any third-party resource; faces come from the system stack and icons are inline SVG.
- **Don't** set times, versions, counts or states in monospace; reserve mono for text a terminal would accept or emit.
- **Don't** dim content by opacity to mean "not in force"; use ink-faint text and a dashed rule so it stays legible.
- **Don't** animate sideways, scale, fade or stagger; one axis, 180ms, one ease.
- **Don't** use 0 radius, pill radius, borders thicker than 1px, uppercase tracking, or a second neutral ramp.

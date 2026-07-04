# TUI Onboarding — the human front door

> Design doc · 2026-07-04 · epic filed in beads (see `bd show` for the id in
> the epic's tracker entry). Scoped exception to the TUI freeze — see
> `planning/skill-cli-boundary.md` §11 (amended).

## 1. Position

The TUI is **the human front door for the rare, high-judgment moments** —
declaring taste, choosing sources, approving rules — and nothing else. Agents
keep their own front doors (MCP, hooks, generated context, `wh init --claude`).
Users should rarely need the TUI; when they do (chiefly: first run), it should
be world-class.

Why this is TUI work and not CLI/skill work: onboarding is choice-heavy and
exploratory (a bad fit for flags), and it is **pre-skill** — you cannot
delegate taste-setup to an agent you haven't configured yet.

**Non-negotiable discipline (from the boundary doc):** every screen is a skin
over existing deterministic oracles (`detect`, `corpus`/packs, `sources`,
`extract submit`, `rules approve`, `check::run`, `generate_context`,
`triggers`/`onboard`). Zero business logic in `src/tui/`. Anything the wizard
needs that doesn't exist becomes a **CLI oracle first** (usable headlessly),
then a screen.

## 2. Design principles

1. **Two paths.** *Express*: detect → accept matched starter packs → review →
   payoff — target ≤ 90 seconds. *Curated*: the full tour. Express is the
   default; Curated is the offer.
2. **Progress is derived, never stored.** The "Setup: 3/5" checklist is
   computed from real artifacts (packs present? sources subscribed? context
   generated? hooks installed? rules approved?) — no parallel onboarding-state
   file to drift. Resumable and re-enterable by construction. One deliberate
   exception: **"skip" persists** as a real-config dismissal key
   (`setup: { dismissed: true }` in `whetstone.yaml` — plain YAML, visible,
   reversible), so a user who declines isn't re-offered the wizard forever.
3. **Nothing becomes enforceable without review.** Two mechanics, one
   principle: (a) **pack imports** (starter/resource/archetype packs ship
   `approved: true` and merge via `extends`) are reviewed **pre-import** — the
   review screen shows every rule (citation + goldens) and *confirm* is what
   writes the `extends` entry, with per-rule opt-outs landing as `deny`
   entries; (b) **extracted/captured/inferred rules** follow the normal
   `candidate → approve` lifecycle in the same screen. Either way, zero rules
   take effect without having been on screen. Bulk actions make this fast,
   never skipped.
4. **Preview before commit.** Selecting a pack shows its live consequence —
   "14 rules · 37 hits on your repo right now" — via a dry-run scan against
   the candidate pack before it is imported.
5. **The payoff is the point.** Onboarding ends with a first scan of *their*
   code, the adherence score, generated context, and optional agent wiring.
   The metric is **time-to-first-catch**, not steps completed.
6. **One state, two front doors.** The wizard and `wh init --claude` produce
   identical artifacts (whetstone.yaml `extends`, packs, context, hooks,
   `.mcp.json`). Either front door can inspect and extend what the other did.
   The consent moment differs by door and we say so honestly: the human door
   reviews before `extends` is written; the agent door is delegated consent —
   so `wh init --claude` prints "imported N pre-verified rules — review them
   anytime: run `wh` (Review screen)".
7. **Judgment is a handoff, not a feature.** Where inference is needed
   ("propose taste from my code"), the TUI offers an explicit agent handoff
   (the capture skill) and receives results back as candidates — it never
   infers anything itself.

## 3. The flow

```
ENTRY  wh (interactive, project not onboarded) → Setup home
       "Setup: 0/5 · [E]xpress · [C]urated · [S]kip"

DETECT      deps list (detect oracle) · matched starter packs highlighted
  ├─ EXPRESS  accept all matches → REVIEW (pre-import) → PAYOFF
  │           └─ ZERO MATCHES (the common case beyond the 10-dep corpus):
  │              route to famous-resources packs (language-level) + the INFER
  │              handoff — never dead-end; PAYOFF has designed zero-findings copy
  └─ CURATED
      PACKS & RESOURCES   bundled corpus packs + famous-resources packs
                          per-pack live preview (rules count · hits now)
      SOURCES & WATCHES   per-dep source selection, changelog watch toggles,
                          defaults-first (official docs auto-on), bulk actions
      PERSONAS/ARCHETYPES [phase 2 — separate epic; artifact-backed bundles]
      INFER (optional)    agent handoff: "propose taste from this repo" →
                          returns candidates into REVIEW
      CONFLICTS           same-id collisions + formatter-option conflicts
                          across selected layers → pick winners → overrides/deny
      REVIEW & APPROVE    rule + citation + goldens · per-rule or bulk-by-
                          confidence · deny = delete
      PAYOFF              first scan + adherence + context generated +
                          "wire your agent now?" (init --claude pieces)
```

Every step is skippable; quitting mid-flow loses nothing (principle 2).

## 4. Oracle gaps to close first (CLI before TUI, per the boundary)

| Gap | Oracle addition |
|---|---|
| Pack preview | `wh scan --with-pack <file> --json` — scan as if the pack were imported, without touching config. Deterministic; also useful headlessly and to agents. |
| Conflict listing | `wh config conflicts --json` — surface same-rule-id collisions and formatter-option conflicts across the configured + proposed layers (mostly exposing existing merge warnings as a stable JSON shape). |
| Setup status | `wh status --setup --json` (or a small `wh onboard status`) — the derived checklist (packs/sources/context/hooks/approved-rules present?) so the TUI, the skill, and Janitor read the same truth. |

## 5. Content workstream (separate from TUI code)

**Famous-resources packs** (phase 1): 3–5 packs extracted from public, citable,
versioned style guides (e.g. Google Python Style Guide, Airbnb JS/TS, Rust API
Guidelines, Effective Go) through the normal extraction + `wh eval` pipeline —
every rule doc-cited, golden-verified, `last_validated` stamped. These are
corpus work, not UI work, and ship independently.

**Personas/archetypes** (phase 2, separate epic): a persona is a **curated
source bundle of real artifacts** (published style guides, repos' lint configs
and CONTRIBUTING.md, essays) fed through the same pipeline — *artifacts, not
aura*. **Phase-2 v1 ships archetypes only** ("stdlib minimalist",
"strict-typing functionalist"). Named-person bundles require the person's
**consent or legal review** — a linkable public corpus is necessary but *not
sufficient* — and when they exist they are framed as "sources: X's published
writings", never style impersonation. X/social posts are weak provenance and
are not acceptable as sole sources. Every rule still cites a real URL and
passes the eval bar.

## 6. What this is not

- Not an unfreeze: the §11 exception covers **onboarding + review/approve**
  only. Dashboard/debt/etc. stay frozen.
- Not a second brain: no inference, no fetching, no rule synthesis in the TUI.
- Not a wizard-only feature: each oracle addition must be independently useful
  to agents and scripts (that's the test that it belongs in the CLI).

## 7. Success criteria

- Express path (matched packs): fresh repo → reviewed + imported starter rules
  + first scan + context in ≤ 90 seconds, ≤ 12 keypresses (keypresses are the
  CI proxy; the 90s is a manual QA target).
- Zero-match repo (e.g. Django/Vue): still reaches review + payoff via the
  resources/INFER routing — no dead-end, and the zero-findings payoff state is
  designed, not an anticlimax.
- Wizard and `wh init --claude` byte-equivalent artifacts on the same repo
  (integration-tested).
- No TUI-only state files; killing the TUI at any step leaves valid config.
- Every imported rule passed through review; zero silently-approved rules.
- Preview numbers match a subsequent real scan (same oracle, same numbers).

---
record: "feature.changelog"
area: "Dashboard"
sweep_order: 4
serves: ["mission.project"]
proven_by: ["standard.changelog"]
entry_points: ["assets/dashboard/views.js", "src/history.rs", "src/projection.rs", "assets/dashboard/index.html", "assets/dashboard/app.js"]
drive_steps: ["open /", "click #tab-changelog", "expect .entry", "expect text=Foundations established", "expect #cl-q", "expect #changelog a.export", "screenshot changelog"]
---

# Changelog

The changelog is the project journal, newest first: every accepted change and every check run, with search, an as-of view, exact records behind disclosure and a decision trail export.

## Sub-features

- `journal-entries` groups entries by day with human titles and status.
- `journal-search` filters by title, summary or area.
- `journal-export` downloads the decision trail as a show-me-your-work TSV.

## How to get to it (user POV)

- Choose the `Changelog` tab.
- Choose `Latest change` at the foot of the Dashboard; it opens the Changelog.
- Run `wh dash --trail` for the same trail as TSV.

## Driving it with drive.mjs

Preconditions:

- `node whetstone/verify/drive.mjs doctor --json` reports `"ok": true` for a fresh build.
- The agreement is established, so the journal holds at least the onboarding entry.

- **Open the changelog.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-changelog" "expect .entry" --json`. At least one journal entry renders.
- **Find the founding decision.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-changelog" "expect text=Foundations established" --json`. The onboarding entry is listed.
- **Proof.** The journal, its search box and the trail export render. Run `wh check --feature feature.changelog`. The receipt names the screenshot.

## Gotchas

- Checks are hidden by default; choose `All` to see verification entries.
- The export link downloads `whetstone-decisions.tsv`; the header row is `ts phase decision why evidence result`.

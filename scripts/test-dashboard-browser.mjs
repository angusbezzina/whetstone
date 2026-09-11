#!/usr/bin/env node
// Synthetic browser proof of the dashboard assets against controlled
// projections: onboarding gate, four views, honest states, safe rendering,
// inline edit and review flows, keyboard, theme, errors and widths.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { join, resolve } from "node:path";
import { delay, launch } from "./cdp.mjs";

const root = resolve(import.meta.dirname, "..");
const asset = (name, type) => [type, readFileSync(join(root, "assets/dashboard", name))];
const assets = {
  "/": asset("index.html", "text/html; charset=utf-8"),
  "/app.css": asset("app.css", "text/css; charset=utf-8"),
  "/views.css": asset("views.css", "text/css; charset=utf-8"),
  "/app.js": asset("app.js", "text/javascript; charset=utf-8"),
  "/views.js": asset("views.js", "text/javascript; charset=utf-8"),
  "/edit.js": asset("edit.js", "text/javascript; charset=utf-8"),
};

const XSS = '<img src=x onerror="globalThis.whetstoneXss=true">';
const label = (tone, text) => ({ tone, label: text });
const entry = (id, kind, title, extra = {}) => ({
  id, kind, title, detail: [], outcomes: [], version: 1, lifecycle: "accepted", pending: null,
  state: null, owner: "Owner", changed_at: "2026-09-10T10:00:00Z",
  edit: { kind: kind === "standard" ? "standard" : kind, record_id: id, content: title, desired_outcome: null, review_triggers: null, definition: null },
  ...extra,
});

function view(mode) {
  const established = mode !== "fresh";
  const gates = [
    entry("standard.journey", "standard", "The dashboard journey is proven", {
      detail: [{ key: "mechanism", value: "Drive", code: false }, { key: "command", value: "prove feature.dashboard", code: true }],
      state: label("fail", "fail · 2"),
    }),
    entry("standard.tests", "standard", "Rust tests pass", {
      detail: [{ key: "mechanism", value: "Test", code: false }, { key: "command", value: "cargo test", code: true }],
      state: label("warn", "pass · stale"),
    }),
    entry("standard.draft", "standard", "No raw regex as enforcement", { lifecycle: "draft", state: label("draft", "draft · not run") }),
  ];
  const feature = {
    ...entry("feature.dashboard", "feature", "Dashboard", { state: label("fail", "fail · 2") }),
    area: "Dashboard", sweep_order: 1, summary: "Mission, attention, metrics and gates at a glance",
    user_path: "Run wh dash and read the default view.", proof: "The mission headline and attention panel render.",
    sub_features: ["attention: one primary action"], drive_steps: ["open /", "expect #mission-line"],
    gotchas: [], entry_points: ["assets/dashboard/"],
    serves: [{ id: "mission.project", kind: "mission", title: "Own the outer loop", resolved: true }],
    constrained_by: [], proven_by: [{ id: "standard.journey", kind: "standard", title: "The dashboard journey is proven", resolved: true }],
    proof_state: label("fail", "fail · 2"), drift: ["assets/dashboard/app.js"],
  };
  const brief = "gate      The dashboard journey is proven (v1, must)\nrecheck   wh check --rule standard.journey";
  return {
    established,
    header: { project: "browser-fixture", agreement: established ? "owner approved" : "not initialised", visibility: "private", drafts: established ? 1 : 0, team: "not configured" },
    onboarding: {
      decisions: [["mission", "Mission"], ["desired outcome", "Desired outcome"], ["core values", "Core values"], ["implementation philosophy", "Engineering philosophy"], ["accountable owner", "Accountable owner"], ["initial safeguard", "First gate"], ["initial safeguard scope", "Gate scope"], ["revision triggers", "Review triggers"]]
        .map(([key, text]) => ({ key, label: text, hint: "decide it", done: false })),
      missing: [], command: "wh init",
    },
    mission: established ? entry("mission.project", "mission", XSS + "Own the outer loop", { outcomes: ["Routine drift is repaired"] }) : null,
    stages: established ? {
      mission: [entry("mission.project", "mission", XSS + "Own the outer loop", { outcomes: ["Routine drift is repaired"] })],
      values: [entry("value.core", "value", "Evidence before assertion", { pending: { version: 2, title: "Evidence always", proposal: "proposal.a" } })],
      metrics: [entry("metric.rework", "metric", "Accepted work needing rework", {
        detail: [{ key: "target", value: "decrease to 5%", code: false }, { key: "window", value: "30 days", code: false }],
        state: label("warn", "not observed"),
      })],
      rules: [entry("philosophy.implementation", "philosophy", "Typed services own deterministic work")],
      gates,
    } : { mission: [], values: [], metrics: [], rules: [], gates: [] },
    features: established ? [feature] : [],
    checks: {
      last_complete: established ? { at: "2026-09-10T10:05:00Z", tally: { pass: 0, fail: 1, unknown: 1 } } : null,
      gates: established ? [
        { id: "standard.journey", name: "The dashboard journey is proven", strength: "must", mechanism: "Drive", command: "prove feature.dashboard", eligible: true, result: label("fail", "fail · 2"), last_run: "2026-09-10T10:05:00Z", current: true, summary: "Driving did not reach the proof.", failures: [{ location: "step 2: expect #mission-line", message: XSS + "missing" }, { location: "page", message: "page errors" }], recheck: "wh check --rule standard.journey", brief, evidence: [{ kind: "whetstone_evidence", locator: "run1/standard_journey.drive/01-home.png", digest: null }, { kind: "whetstone_evidence", locator: "run1/standard_journey.log", digest: null }], feature: null },
        { id: "standard.tests", name: "Rust tests pass", strength: "must", mechanism: "Test", command: "cargo test", eligible: true, result: label("warn", "pass · stale"), last_run: "2026-09-09T10:05:00Z", current: false, summary: "Passed in 3.0s.", failures: [], recheck: "wh check --rule standard.tests", brief: null, evidence: [], feature: null },
        { id: "standard.draft", name: "No raw regex as enforcement", strength: "should", mechanism: "AST query", command: "no-regex", eligible: false, result: label("draft", "draft · not run"), last_run: null, current: false, summary: null, failures: [], recheck: "wh check --rule standard.draft", brief: null, evidence: [], feature: null },
      ] : [],
      advisory: 1, receipts: [], driver: { configured: true, path: "whetstone/verify/drive.mjs", label: "verification driver present" },
    },
    attention: established ? [
      { priority: 0, tone: "fail", kind: "agent_repair", kind_label: "agent repair", title: "Failing gate: The dashboard journey is proven", text: "2 failures in the latest check.", actor: "your coding agent", next: "Hand the brief to the agent.", route: "checks", focus: "standard.journey", action_label: "Open the failing gate", agent_instruction: brief },
      { priority: 1, tone: "warn", kind: "stale_result", kind_label: "not current", title: "Not current: Rust tests pass", text: "Stale.", actor: "you or your agent", next: "Run it again.", route: "checks", focus: "standard.tests", action_label: "Open checks", agent_instruction: null },
      { priority: 2, tone: "muted", kind: "review", kind_label: "review", title: "1 local draft awaits your review", text: "Private.", actor: "Owner", next: "Accept or withdraw it.", route: "changelog", focus: null, action_label: "Open the changelog", agent_instruction: null },
    ] : [],
    latest_change: established ? { title: "Core value revised: Evidence always", at: "2026-09-10T10:00:00Z" } : null,
    skill: { rendered: false, current: false, hosts: [], rendered_at: null, label: label("warn", "not generated") },
  };
}

const journal = [
  { id: "c2", recorded_at: "2026-09-10T10:00:00Z", kind: "decision", title: "Core value revised: Evidence always", version: "v1 → v2", status: label("draft", "draft"), owner: "Owner", area: "values", summary: XSS + "Sharper wording", note: null, proposal: "proposal.a", records: [{ record: { id: "value.core" } }] },
  { id: "check:1", recorded_at: "2026-09-10T10:05:00Z", kind: "verification", title: "Checks ran: 0 pass · 1 fail · 1 unknown", version: null, status: label("fail", "fail"), owner: null, area: "checks", summary: "Gates: standard.journey", note: null, proposal: null, records: [] },
  { id: "c1", recorded_at: "2026-09-01T09:00:00Z", kind: "decision", title: "Foundations established", version: "revision 1", status: label("pass", "accepted"), owner: "Owner", area: "mission", summary: "Recorded as one wh init operation.", note: null, proposal: null, records: [] },
];

// A long history: the journal pages locally, 100 entries at a time.
const longJournal = Array.from({ length: 130 }, (_, index) => ({ id: `long-${index}`, recorded_at: new Date(Date.UTC(2026, 8, 10, 12, 0) - index * 60_000).toISOString(), kind: "decision", title: `Decision ${index}`, version: "revision 1", status: label("pass", "accepted"), owner: "Owner", area: "rules", summary: `Entry ${index}`, note: null, proposal: null, records: [] }));

let mode = "established";
const commands = [];
function envelope() {
  return { schema: "whetstone.command-response.v1", state: "success", summary: "ok", data: { current: view(mode), changelog: mode === "fresh" ? [] : mode === "long" ? longJournal : journal, history: { snapshot_digest: "sha256:x", decision_history: { next: null } }, sync: { shared_store: "/repo", team_active: { "standard.journey": "standard.journey@1#sha256:x" }, required_workflow: false } } };
}

const server = createServer((request, response) => {
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    const send = (status, body, type = "application/json") => {
      response.writeHead(status, { "content-type": type, "cache-control": "no-store" });
      response.end(typeof body === "string" || Buffer.isBuffer(body) ? body : JSON.stringify(body));
    };
    if (request.method === "GET" && assets[request.url]) return send(200, assets[request.url][1], assets[request.url][0]);
    if (request.method === "GET" && request.url.startsWith("/api/evidence/")) {
      return send(200, Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=", "base64"), "image/png");
    }
    if (request.url === "/session/bootstrap") return send(200, { csrf_token: "csrf-1" });
    if (request.url === "/session/edit") return send(200, { mode: "edit" });
    if (request.url === "/api/inspect") {
      if (mode === "error") return send(500, { state: "unknown", summary: "Synthetic inspection failure." });
      return send(200, envelope());
    }
    if (request.url === "/api/command") {
      const body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      assert.equal(request.headers["x-whetstone-csrf"], "csrf-1", "mutations carry the CSRF header");
      commands.push(body);
      if (body.workflow === "change" && body.review) {
        return send(200, body.preview
          ? { state: "needs_decision", summary: "Review.", data: { preview_only: true, candidates: [{ title: "Evidence always", version: 2, replaces: 1 }] } }
          : { state: "success", summary: "The draft was accepted locally.", data: {} });
      }
      if (body.workflow === "change" && body.expected_revision == null) {
        return send(200, { state: "needs_input", summary: "Base ready.", expected_revision: 3, resume_token: "tok-3", blocking_questions: ["Base revision 3"], data: {} });
      }
      if (body.workflow === "change" && body.preview) {
        assert.equal(body.expected_revision, 3);
        assert.equal(body.resume_token, "tok-3");
        return send(200, { state: "needs_decision", summary: "Review.", data: { preview_only: true, base_revision: 3, diff: { before: { record: { description: "Evidence before assertion" } }, after: { record: { description: body.content } } } } });
      }
      if (body.workflow === "change") return send(200, { state: "success", summary: "The private draft was recorded.", data: {} });
      if (body.workflow === "check") return send(200, { state: "violated", summary: "One or more required checks failed.\nFAIL gate", data: {} });
      return send(200, { state: "needs_input", summary: "More input.", data: {} });
    }
    send(404, "not found", "text/plain");
  });
});
await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
const origin = `http://127.0.0.1:${server.address().port}`;

const browser = await launch();
if (!browser) {
  console.log("SKIP dashboard browser test: Chrome/Chromium is unavailable");
  server.close();
  process.exit(77);
}
const { cdp } = browser;
const click = (selector) => cdp.evaluate(`(() => { const n = document.querySelector(${JSON.stringify(selector)}); if (!n) throw new Error("missing ${selector.replaceAll('"', "'")}"); n.click(); return true; })()`);
const count = (selector) => cdp.evaluate(`document.querySelectorAll(${JSON.stringify(selector)}).length`);
const text = (selector) => cdp.evaluate(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? null`);
let checks = 0;
const check = (condition, message) => { assert.ok(condition, message); checks += 1; };

try {
  // Fresh project: onboarding only, later views gated.
  mode = "fresh";
  await cdp.navigate(`${origin}/`);
  await cdp.waitFor("document.querySelector('.onboard h1')", "onboarding did not render");
  check(await count("#dashboard .panel") === 0, "fresh dashboard shows no metric, gate or attention panels");
  check(await count(".onboard li") === 8, "eight owner decisions are listed");
  check(await cdp.evaluate("document.querySelector('#tab-checks').getAttribute('aria-disabled') === 'true' && document.querySelector('#tab-changelog').getAttribute('aria-disabled') === 'true'"), "checks and changelog are gated before init");
  await click("#tab-checks");
  check(await cdp.evaluate("document.querySelector('#dashboard').hidden === false"), "a gated view does not open");
  check(await count(".onboard .btn.primary") === 0, "without edit capability there is no establish action");

  // Established with edit capability.
  mode = "established";
  await cdp.navigate(`${origin}/#bootstrap=synthetic`);
  await cdp.waitFor("document.querySelector('#mission-line')", "established dashboard did not render");
  check(await cdp.evaluate("location.hash === ''"), "the bootstrap fragment is removed from the URL");
  check(await cdp.evaluate("!globalThis.whetstoneXss && document.querySelector('#mission-line').textContent.startsWith('<img')"), "hostile record text renders as text");
  check(await count("#dashboard .attn .btn.primary") === 1, "exactly one primary action on the dashboard");
  check(await count("#dashboard .attn.more") === 2, "remaining attention items are secondary rows");
  check(await cdp.evaluate("[...document.querySelectorAll('#dashboard .state.pass')].every(n => !/unknown|not|stale|draft/.test(n.textContent))"), "unknown, stale and draft never render as pass");
  check(await count("#dashboard .state.warn") >= 2, "not observed and stale states are amber");
  check(await text("#draft-count") === "1" && await text("#draft-word") === "draft", "the draft count is exact and pluralised");

  // Attention routes to the failing gate with its brief open.
  await click("#attention-primary .btn.primary");
  await cdp.waitFor("!document.querySelector('#checks').hidden && document.querySelector('#det-standard\\\\.journey.open')", "attention did not open the failing gate");
  check(await cdp.evaluate("document.activeElement?.dataset.id === 'standard.journey'"), "focus moves to the failing gate");
  check(await cdp.evaluate("document.querySelector('#det-standard\\\\.journey pre.brief').textContent.includes('recheck')"), "the repair brief is shown");
  check(await cdp.evaluate("document.querySelector('#det-standard\\\\.journey .evid img')?.getAttribute('src') === '/api/evidence/run1/standard_journey.drive/01-home.png' && document.querySelector('#det-standard\\\\.journey .evid a[href$=\\'.log\\']') !== null"), "driven proofs are inspectable with their artifacts");
  check(await cdp.evaluate("document.querySelector('#checks').textContent.includes('Team policy') && document.querySelector('#checks').textContent.includes('required check not installed')"), "team activation state is shown");
  check(await cdp.evaluate("!globalThis.whetstoneXss"), "failure messages render as text");
  check(await cdp.evaluate("document.querySelector('#gate-standard\\\\.draft small').textContent.includes('not eligible')"), "draft gates are marked not eligible");
  await cdp.evaluate("document.querySelector('#checks select').value = 'changed'; true");
  await click("#run-btn");
  await cdp.waitFor("document.querySelector('#announce').textContent.includes('One or more required')", "run checks did not report");
  check(commands.at(-1).workflow === "check" && commands.at(-1).mode === "changed", "run checks sends the selected mode");

  // Foundations: five stages plus features, inline edit, exact review.
  await click("#tab-foundations");
  await cdp.waitFor("document.querySelector('#st-gates')", "foundations did not render");
  check(await cdp.evaluate("[...document.querySelectorAll('#foundations .panel .ph h2')].map(h => h.firstChild.textContent).join('|') === 'Mission|Core values|Key metrics|Rules & guidelines|Gates|Features'"), "stages render in the agreed order");
  check(await cdp.evaluate("document.querySelector('[data-id=\"value.core\"] .ver').textContent.includes('draft v2 pending')"), "a pending draft is labelled while the accepted record stays in force");
  check(await cdp.evaluate("document.querySelector('[data-id=\"value.core\"] .c').textContent === 'Evidence before assertion'"), "accepted content stays in force");
  check(await cdp.evaluate("!document.body.innerText.includes('null')"), "no literal null is rendered");
  await click('[data-id="value.core"] .e button');
  await cdp.waitFor("document.querySelector('.editor.open form')", "editor did not open inline");
  check(await count("dialog[open]") === 0, "editing opens inline, not in a modal");
  await cdp.evaluate(`(() => { const f = document.querySelector('.editor.open form'); f.querySelector('[name=content]').value = 'Evidence always'; f.querySelector('[name=rationale]').value = 'Sharper'; f.requestSubmit(); return true; })()`);
  await cdp.waitFor("document.querySelector('.review .diff .after')", "the exact review did not render");
  check(await cdp.evaluate("document.querySelector('.review .diff .after').textContent.includes('Evidence always')"), "the review shows the exact after state");
  check(await cdp.evaluate("document.activeElement?.id === 'record-draft'"), "focus moves to the confirm action");
  await click("#record-draft");
  await cdp.waitFor("document.querySelector('#announce').textContent.includes('private draft')", "recording did not announce");
  check(commands.filter((c) => c.workflow === "change" && !c.review).length === 3, "probe, preview and record are three bound calls");
  check(await cdp.evaluate("document.querySelector('.feat details summary').textContent.includes('Runbook')"), "features carry their runbook");
  check(await cdp.evaluate("document.querySelector('.feat dl').textContent.includes('Changed since proof')"), "feature drift is shown in the runbook");

  // Changelog: decisions by default, accept a draft through review.
  await click("#tab-changelog");
  await cdp.waitFor("document.querySelector('.entry')", "changelog did not render");
  check(await count(".entry") === 2, "verification entries are hidden by default");
  check(await cdp.evaluate("document.querySelector('#changelog a.export')?.getAttribute('href') === '/api/trail.tsv'"), "the decision trail exports as TSV");
  await cdp.evaluate("[...document.querySelectorAll('.filter button')].find(b => b.textContent === 'All').click(); true");
  check(await count(".entry") === 3, "All shows verification entries too");
  check(await cdp.evaluate("!globalThis.whetstoneXss"), "journal summaries render as text");
  await cdp.evaluate("[...document.querySelectorAll('.entry .act .btn')].find(b => b.textContent === 'Accept').click(); true");
  await cdp.waitFor("document.querySelector('#confirm-review')", "accept did not show its review");
  await click("#confirm-review");
  await cdp.waitFor("document.querySelector('#announce').textContent.includes('accepted')", "accept did not announce");
  check(commands.some((c) => c.review?.verdict === "accept" && c.preview === false), "accept records the review after preview");

  // Keyboard and theme.
  await cdp.evaluate("document.querySelector('#tab-dashboard').focus(); document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })); true");
  check(await cdp.evaluate("document.activeElement.id === 'tab-changelog'"), "End moves to the last tab");
  await cdp.evaluate("document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })); true");
  check(await cdp.evaluate("document.activeElement.id === 'tab-dashboard'"), "arrow keys wrap around the tabs");
  await click('.theme button[data-theme="dark"]');
  check(await cdp.evaluate("document.documentElement.dataset.theme === 'dark' && localStorage.getItem('wh-theme') === 'dark'"), "the theme toggle applies and remembers dark");
  await click('.theme button[data-theme="system"]');
  check(await cdp.evaluate("!document.documentElement.hasAttribute('data-theme')"), "system theme follows the OS");

  // Widths.
  for (const width of [320, 390, 768, 1024]) {
    await cdp.viewport(width);
    for (const tab of ["dashboard", "foundations", "checks", "changelog"]) {
      await click(`#tab-${tab}`);
      await delay(80);
      const overflow = await cdp.evaluate("document.documentElement.scrollWidth - window.innerWidth");
      check(overflow <= 1, `${tab} fits at ${width}px (overflow ${overflow})`);
    }
  }

  // Long history: the newest entries first, older ones appended in place.
  mode = "long";
  await cdp.navigate(`${origin}/`);
  await cdp.waitFor("document.querySelector('#mission-line')", "long dashboard did not render");
  await click("#tab-changelog");
  await cdp.waitFor("document.querySelector('.entry')", "long changelog did not render");
  check(await count(".entry") === 100 && await cdp.evaluate("document.querySelector('.entry').textContent.includes('Decision 0')"), "the first page holds the newest 100 entries");
  const before = commands.length;
  await click("#cl-more");
  check(await count(".entry") === 130 && !(await cdp.evaluate("Boolean(document.querySelector('#cl-more'))")), "Load more appends the older entries");
  check(commands.length === before, "Load more pages locally without a new command");

  // Inspection failure: no health claim.
  mode = "error";
  await cdp.viewport(1024);
  await cdp.navigate(`${origin}/`);
  await cdp.waitFor("document.querySelector('#dashboard .empty')", "the error state did not render");
  check(await cdp.evaluate("document.querySelector('#dashboard .empty').textContent.includes('no health claim')"), "an unavailable projection makes no claim");

  const external = cdp.requests.filter((url) => !url.startsWith(origin) && !url.startsWith("data:"));
  check(external.length === 0, `no external requests: ${external.join(", ")}`);
  const errors = cdp.errors.filter((error) => !error.includes("500"));
  check(errors.length === 0, `no page errors: ${errors.join("\n")}`);
  console.log(`PASS synthetic dashboard (${checks} assertions)`);
} finally {
  await browser.close();
  server.close();
}

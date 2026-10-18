#!/usr/bin/env node
// Synthetic browser proof of the dashboard assets against controlled
// projections: gated onboarding, the four views, honest states, safe
// rendering, the exact bodies every mutation sends, keyboard, theme, errors
// and widths.

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
  id, kind, title, detail: [], version: 1, lifecycle: "accepted", pending: null,
  state: null, owner: "Owner", changed_at: "2026-09-10T10:00:00Z",
  edit: { kind, record_id: id, content: title, definition: extra.definition ?? null },
  ...extra,
});
const example = (input, expected, reason) => ({ input, expected, reason, path: null });
function rule(id, title, strength, enforcer, command, extra = {}) {
  const definition = { type: "rule", strength, enforcer: extra.enforcerRecord || { kind: "test", command }, examples: extra.examples || [], source: { kind: "owner" }, paths: [] };
  return {
    ...entry(id, "rule", title, { definition, state: extra.result || label("pass", "pass"), lifecycle: extra.lifecycle || "accepted", pending: extra.pending || null }),
    strength, enforcer, command,
    runs_at: extra.runs_at || "pre-push and CI", sweep_class: extra.sweep_class || "discuss", origin_lesson: null, source: "owner", paths: [], examples: extra.examples || [],
    result: extra.result || label("pass", "pass"),
  };
}
const gate = (id, name, strength, mechanism, command, result, extra = {}) => ({
  id, name, strength, mechanism, command, eligible: extra.eligible ?? true, result,
  last_run: extra.last_run === undefined ? "2026-09-10T10:05:00Z" : extra.last_run, current: true, summary: extra.summary ?? null,
  failures: extra.failures || [], recheck: `wh check --rule ${id}`, brief: extra.brief || null, evidence: extra.evidence || [], feature: null, team: "private",
});

const STEPS = [["tools", "Beads and pstack", true], ["mission", "Mission", false], ["principles", "Principles", true], ["exemplars", "Exemplars", true], ["rules", "Rules", false], ["gates", "Gates", false]];
const CATALOGUE = [
  { id: "laziness-protocol", group: "core", rule: "Bias toward deletion and the smallest change.", enforceable: "question", suggestion: "Does this change add code the task did not need?" },
  { id: "prove-it-works", group: "core", rule: "Verify against the real artifact before declaring done.", enforceable: "mechanical", suggestion: null },
  { id: "boundary-discipline", group: "core", rule: XSS + "Concentrate guards at system boundaries.", enforceable: "question", suggestion: null },
];
const STARTERS = [
  { id: "rule.prove-it-works", title: "Prove it works before done", detected: "cargo test was detected", strength: "must", enforcer: "Test", examples: [example("ran the tests", "pass", "proved")], accepted: false },
];
const brief = "rule      No TODO comments (v1, should)\nrecheck   wh check --rule rule.no-todo";

function view(mode) {
  const established = mode !== "fresh";
  const rules = [
    rule("rule.no-unsafe", "No unsafe blocks.", "must", "AST query", "(unsafe_block) @match", { runs_at: "pre-commit, pre-push and CI", examples: [example("pub fn run(p: *const u8) -> u8 { unsafe { *p } }", "flag", "An unsafe block.")], enforcerRecord: { kind: "ast", query: "(unsafe_block) @match", language: "rust" } }),
    rule("rule.tests", "Rust tests pass.", "must", "Test", "cargo test", { result: label("warn", "pass · stale") }),
    rule("rule.no-todo", XSS + "No TODO comments.", "should", "Test", "false", { result: label("fail", "fail · 1"), pending: { version: 2, title: "No TODO or FIXME", proposal: "proposal.a" } }),
    rule("rule.plain-words", "Name things in plain words.", "advisory", "Validator", "scripts/plain-words", { result: label("muted", "advisory"), enforcerRecord: { kind: "validator", command: "scripts/plain-words" } }),
  ];
  const feature = {
    ...entry("feature.dashboard", "feature", "Dashboard", { state: label("fail", "fail · 2") }),
    area: "Dashboard", sweep_order: 1, summary: "Mission, attention and rules at a glance", user_path: "Run wh dash.", proof: "The mission headline renders.",
    sub_features: [], drive_steps: ["open /"], gotchas: [], entry_points: ["assets/dashboard/"],
    serves: [{ id: "mission.project", kind: "mission", title: "Own the outer loop", resolved: true }], constrained_by: [], proven_by: [],
    proof_state: label("fail", "fail · 2"), drift: ["assets/dashboard/app.js"],
  };
  return {
    established,
    header: { project: "browser-fixture", agreement: established ? "owner approved" : "not initialised", visibility: "private", drafts: established ? 1 : 0, team: "not shared" },
    onboarding: {
      steps: STEPS.map(([key, text, optional]) => ({ key, label: text, hint: `hint for ${key}`, done: established && !["gates", "exemplars", "tools"].includes(key), optional })),
      missing: established ? [] : ["mission", "rules"], command: "wh init", catalogue: CATALOGUE, catalogue_version: "0.15.5", starters: STARTERS,
      legacy: established ? [{ id: "value.core", kind: "core value", text: "Evidence before assertion", migrated: false }] : [],
      tools: [{ tool: "bd", state: "ok", found: "1.3.0", detail: "Beads is installed.", fix: null }, { tool: "pstack:agents", state: "missing", found: null, detail: "pstack skills missing.", fix: "npx skills add pstack" }],
    },
    mission: established ? entry("mission.project", "mission", XSS + "Own the outer loop") : null,
    principles: established ? [
      entry("principle.prove-it-works", "principle", "Verify against the real artifact.", { detail: [{ key: "source", value: "pstack prove-it-works (0.15.5)", code: false }], definition: { type: "principle", pstack: "prove-it-works", rationale: null } }),
    ] : [],
    rules: established ? rules : [],
    features: established ? [feature] : [],
    checks: {
      last_complete: established ? { at: "2026-09-10T10:05:00Z", tally: { pass: 1, fail: 1, unknown: 1 } } : null,
      gates: established ? [
        gate("rule.no-unsafe", "No unsafe blocks.", "must", "AST query", "(unsafe_block) @match", label("pass", "pass"), { summary: "No unsafe block in 3 file(s)." }),
        gate("rule.tests", "Rust tests pass.", "must", "Test", "cargo test", label("warn", "quarantined · flaky"), { summary: "Flaky: failed once, passed on retry." }),
        gate("rule.no-todo", "No TODO comments.", "should", "Test", "false", label("fail", "fail · 1"), { failures: [{ location: "exit code 1", message: XSS + "$ false" }], brief, evidence: [{ kind: "whetstone_evidence", locator: "run1/rule_no-todo.log", digest: null }] }),
        gate("rule.draft", "No raw regex as enforcement.", "should", "AST query", "(regex)", label("draft", "draft · not run"), { eligible: false, last_run: null }),
      ] : [],
      advisory: 1, receipts: [], driver: { configured: true, path: "whetstone/verify/drive.mjs", label: "verification driver present" },
    },
    attention: established ? [
      { priority: 1, tone: "fail", kind: "agent_repair", kind_label: "agent repair", title: "Failing rule: No TODO comments", text: "1 failure in the latest check.", actor: "your coding agent", next: "Hand the brief to the agent.", route: "checks", focus: "rule.no-todo", action_label: "Open the failing rule", agent_instruction: brief },
      { priority: 2, tone: "muted", kind: "review", kind_label: "review", title: "1 draft awaits your review", text: "Drafts are private.", actor: "Owner", next: "Accept or withdraw each draft in the changelog.", route: "changelog", focus: null, action_label: "Open the changelog", agent_instruction: null },
    ] : [],
    latest_change: established ? { title: "Rule revised: No TODO or FIXME", at: "2026-09-10T10:00:00Z" } : null,
    skill: { rendered: false, current: false, hosts: [], rendered_at: null, label: label("warn", "not generated") },
    hygiene: [],
  };
}

const receipt = (id, recordType, body) => ({ record: { id, record_type: recordType, record: body } });
const journal = [
  { id: "c2", recorded_at: "2026-09-10T10:00:00Z", kind: "decision", title: "Rule revised: No TODO or FIXME", version: "v1 → v2", status: label("draft", "draft"), owner: "Owner", area: "rules", summary: XSS + "Sharper wording", note: null, proposal: "proposal.a", records: [] },
  { id: "check:1", recorded_at: "2026-09-10T10:05:00Z", kind: "verification", title: "Checks ran: 1 pass · 1 fail · 1 unknown", version: null, status: label("fail", "fail"), owner: null, area: "checks", summary: "Rules: rule.no-todo", note: null, proposal: null, records: [
    receipt("verification.gate_a", "verification_receipt", { subject: { stable_id: "gate:rule.no-todo" }, verification: "fail", checked_at: "2026-09-10T10:05:00Z" }),
    receipt("verification.gate_b", "verification_receipt", { subject: { stable_id: "gate:rule.no-unsafe" }, verification: "pass", checked_at: "2026-09-10T10:05:00Z" }),
    receipt("verification.gate_old", "verification_receipt", { subject: { stable_id: "gate:rule.no-todo" }, verification: "fail", checked_at: "2026-09-09T10:05:00Z" }),
  ] },
  { id: "c1", recorded_at: "2026-09-01T09:00:00Z", kind: "decision", title: "Agreement established", version: "revision 1", status: label("pass", "accepted"), owner: "Owner", area: "mission", summary: "Recorded as one wh init operation.", note: null, proposal: null, records: [] },
];
const longJournal = Array.from({ length: 130 }, (_, index) => ({ id: `long-${index}`, recorded_at: new Date(Date.UTC(2026, 8, 10, 12, 0) - index * 60_000).toISOString(), kind: "decision", title: `Decision ${index}`, version: "revision 1", status: label("pass", "accepted"), owner: "Owner", area: "rules", summary: `Entry ${index}`, note: null, proposal: null, records: [] }));

let mode = "established";
const commands = [];
function envelope() {
  return { schema: "whetstone.command-response.v1", state: "success", summary: "ok", data: { current: view(mode), changelog: mode === "fresh" ? [] : mode === "long" ? longJournal : journal } };
}
const agreeRecords = [
  { id: "mission.project", record_type: "mission", record: { statement: "Ship a parser" } },
  { id: "principle.prove-it-works", record_type: "principle", record: { statement: "Verify.", source: { kind: "pstack" } } },
  { id: "rule.prove-it-works", record_type: "rule", record: { statement: "Prove it works.", strength: "must", enforcer: { kind: "test" } } },
];

const server = createServer((request, response) => {
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    const send = (status, body, type = "application/json") => {
      response.writeHead(status, { "content-type": type, "cache-control": "no-store" });
      response.end(typeof body === "string" || Buffer.isBuffer(body) ? body : JSON.stringify(body));
    };
    if (request.method === "GET" && assets[request.url]) return send(200, assets[request.url][1], assets[request.url][0]);
    if (request.method === "GET" && request.url.startsWith("/api/evidence/")) return send(200, "log line", "text/plain");
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
      if (body.workflow === "init" && body.action === "inspect") return send(200, { state: "needs_input", summary: "Inspect.", expected_revision: 0, resume_token: "tok-0", data: {} });
      if (body.workflow === "init" && body.action === "exemplar") return send(200, { state: "needs_decision", summary: "2 rule drafts were recorded from the exemplar.", data: {} });
      if (body.workflow === "init" && body.action === "agree") {
        assert.equal(body.expected_revision, 0);
        assert.equal(body.resume_token, "tok-0");
        return send(200, body.dry_run
          ? { state: "needs_decision", summary: "Dry run.", data: { dry_run: true, records: agreeRecords } }
          : { state: "needs_input", summary: "The agreement is recorded.", data: {} });
      }
      if (body.workflow === "change" && body.review) {
        return send(200, body.preview
          ? { state: "needs_decision", summary: "Review.", data: { preview_only: true, candidates: [{ title: "No TODO or FIXME", version: 2, replaces: 1 }] } }
          : { state: "success", summary: "The draft was accepted.", data: {} });
      }
      if (body.workflow === "change" && body.migrate) return send(200, { state: "success", summary: "Recorded 1 principle draft.", data: {} });
      if (body.workflow === "change" && body.expected_revision == null) {
        return send(200, { state: "needs_input", summary: "Base ready.", expected_revision: 3, resume_token: "tok-3", blocking_questions: ["Base revision 3"], data: {} });
      }
      if (body.workflow === "change" && body.preview) {
        assert.equal(body.expected_revision, 3);
        assert.equal(body.resume_token, "tok-3");
        return send(200, { state: "needs_decision", summary: "Review.", data: { preview_only: true, base_revision: 3, diff: { before: { record: { statement: "before" } }, after: { record: { statement: body.content } } } } });
      }
      if (body.workflow === "change") return send(200, { state: "success", summary: "The private draft was recorded.", data: {} });
      if (body.workflow === "check") return send(200, { state: "violated", summary: "A must rule failed.\nFAIL gate", data: {} });
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
const q = JSON.stringify;
const click = (selector) => cdp.evaluate(`(() => { const n = document.querySelector(${q(selector)}); if (!n) throw new Error("missing " + ${q(selector)}); n.click(); return true; })()`);
const count = (selector) => cdp.evaluate(`document.querySelectorAll(${q(selector)}).length`);
const text = (selector) => cdp.evaluate(`document.querySelector(${q(selector)})?.textContent ?? null`);
const fill = (form, values) => cdp.evaluate(`(() => { const f = document.querySelector(${q(form)}); for (const [k, v] of Object.entries(${q(values)})) { const n = f.querySelector('[name="' + k + '"]'); n.value = v; n.dispatchEvent(new Event("change")); } f.requestSubmit(); return true; })()`);
const last = (predicate) => commands.filter(predicate).at(-1);
const TABS = ["home", "rules", "checks", "changelog"];
let checks = 0;
const check = (condition, message) => { assert.ok(condition, message); checks += 1; };

try {
  // Fresh project, read-only: onboarding only, later views gated.
  mode = "fresh";
  await cdp.navigate(`${origin}/`);
  await cdp.waitFor("document.querySelector('#onboard h1')", "onboarding did not render");
  check(await count("#home .panel") === 0, "fresh home shows no attention or rule panels");
  check(await cdp.evaluate("[...document.querySelectorAll('#onboard .steps li')].map(l => l.dataset.step).join() === 'tools,mission,principles,exemplars,rules,gates'"), "the six onboarding steps are listed in order");
  check(await cdp.evaluate("/npx skills add pstack/.test(document.querySelector('#onboard').textContent) && !/Beads is installed/.test(document.querySelector('#onboard').textContent)"), "missing tools show their fix; installed ones are not listed");
  check(await cdp.evaluate("['checks','changelog'].every(t => document.querySelector('#tab-' + t).getAttribute('aria-disabled') === 'true') && document.querySelector('#tab-rules').getAttribute('aria-disabled') === 'false'"), "checks and changelog are gated before the agreement");
  await click("#tab-checks");
  check(await cdp.evaluate("document.querySelector('#home').hidden === false"), "a gated view does not open");
  check(await count("#start-onboarding") === 0, "without edit capability there is no onboarding action");
  check(await cdp.evaluate("!/eight|core value|metric|foundations/i.test(document.body.innerText)"), "no removed decision, value, metric or foundations language");

  // Fresh project with edit capability: the onboarding form posts the new init body.
  await cdp.navigate(`${origin}/#bootstrap=synthetic`);
  await cdp.waitFor("document.querySelector('#start-onboarding')", "the onboarding action did not render");
  await click("#start-onboarding");
  await cdp.waitFor("document.querySelector('#init-form')", "the onboarding form did not open");
  check(await count("#ob-principles input[name=principle]") === 3, "every catalogue principle is offered");
  check(await cdp.evaluate("!globalThis.whetstoneXss"), "catalogue text renders as text");
  check(await count("#ob-starters input[name=starter]:checked") === 1, "starter rules are preselected");
  await cdp.evaluate("document.querySelector('#init-form [name=exemplar]').value = '../admired'; document.querySelector('#exemplar-btn').click(); true");
  await cdp.waitFor("document.querySelector('#init-form .notice')?.textContent.includes('2 rule drafts')", "the exemplar result did not show");
  check(last((c) => c.action === "exemplar")?.exemplar === "../admired", "an exemplar posts its locator");
  await cdp.evaluate("document.querySelector('#ob-principles input[value=\"prove-it-works\"]').checked = true; true");
  await fill("#init-form", { mission: "Ship a parser", custom_principles: "Prefer boring code\n\nSmall steps" });
  await cdp.waitFor("document.querySelector('#confirm-init')", "the agreement review did not render");
  check(await count("#onboard .review .row") === 3, "the review lists the exact records");
  const agree = last((c) => c.action === "agree");
  check(agree.dry_run === true && agree.mission === "Ship a parser", "the review is a dry run of the agreement");
  check(q(agree.principles) === q(["prove-it-works"]) && q(agree.custom_principles) === q(["Prefer boring code", "Small steps"]), "principles and custom principles are sent as lists");
  check(q(agree.starters) === q(["rule.prove-it-works"]), "only the accepted starters are sent");
  check(!("values" in agree) && !("owner" in agree) && !("desired_outcome" in agree), "no removed onboarding fields are sent");
  mode = "established";
  await click("#confirm-init");
  await cdp.waitFor("document.querySelector('#mission-line')", "confirming did not establish the dashboard");
  check(last((c) => c.action === "agree").dry_run === false, "confirm records the same agreement");

  // Established home: one primary action, rules in force, honest states.
  check(await cdp.evaluate("location.hash === ''"), "the bootstrap fragment is removed from the URL");
  check(await cdp.evaluate("!globalThis.whetstoneXss && document.querySelector('#mission-line').textContent.startsWith('<img')"), "hostile record text renders as text");
  check(await count("#home .btn.primary") === 1, "exactly one primary action on home");
  check(await count("#home .attn.more") === 1, "remaining attention items are secondary rows");
  check(await count("#home-rules .row") === 4, "home lists every rule in force");
  check(await cdp.evaluate("[...document.querySelectorAll('#home .state.pass')].every(n => !/unknown|not|stale|draft/.test(n.textContent))"), "unknown, stale and draft never render as pass");
  check(await text("#draft-count") === "1" && await text("#draft-word") === "draft", "the draft count is exact and pluralised");

  // Attention routes to the failing rule in Checks.
  await click("#attention-primary .btn.primary");
  await cdp.waitFor("!document.querySelector('#checks').hidden", "attention did not open checks");
  check(await cdp.evaluate("document.activeElement?.dataset.id === 'rule.no-todo'"), "focus moves to the failing rule");

  // Checks: quarantined, failures, brief, evidence.
  await click("#tab-checks");
  await cdp.waitFor("document.querySelector('#gate-rule\\\\.no-todo')", "checks did not render");
  check(await cdp.evaluate("document.querySelector('#det-rule\\\\.no-todo').classList.contains('open') && document.querySelector('#det-rule\\\\.no-todo pre.brief').textContent.includes('recheck')"), "a failing rule opens with its repair brief");
  check(await cdp.evaluate("document.querySelector('#det-rule\\\\.no-todo .evid a[href=\"/api/evidence/run1/rule_no-todo.log\"]') !== null"), "evidence is linked");
  check(await cdp.evaluate("document.querySelector('#gate-rule\\\\.tests .state').classList.contains('warn') && /quarantined/.test(document.querySelector('#gate-rule\\\\.tests .state').textContent)"), "a quarantined flaky proof is amber, not green");
  check(await cdp.evaluate("document.querySelector('#gate-rule\\\\.draft small').textContent.includes('not checked')"), "draft rules are marked not checked");
  check(await cdp.evaluate("!globalThis.whetstoneXss"), "failure messages render as text");
  await cdp.evaluate("document.querySelector('#checks select').value = 'staged'; true");
  await click("#run-btn");
  await cdp.waitFor("document.querySelector('#announce').textContent.includes('A must rule failed')", "run checks did not report");
  check(last((c) => c.workflow === "check").mode === "staged", "run checks sends the selected mode");

  // Rules: grouped by strength, enforcer, runs at, examples, edits.
  await click("#tab-rules");
  await cdp.waitFor("document.querySelector('#r-must')", "rules did not render");
  check(await cdp.evaluate("[...document.querySelectorAll('#rules .panel .ph h2')].map(h => h.firstChild.textContent).join('|') === 'Mission|Principles|Must|Should|Advisory|Earlier records|Features'"), "rules render mission, principles, strengths, legacy and features in order");
  check(await count("#r-must .rec") === 2 && await count("#r-should .rec") === 1 && await count("#r-advisory .rec") === 1, "every rule sits under its strength");
  check(await cdp.evaluate("/pre-commit, pre-push and CI/.test(document.querySelector('[data-id=\"rule.no-unsafe\"]').textContent)"), "a rule says where it runs");
  check(await cdp.evaluate("document.querySelector('[data-id=\"rule.no-todo\"] .ver').textContent.includes('draft v2 pending')"), "a pending draft is labelled while the accepted rule stays in force");
  check(await cdp.evaluate("document.querySelector('[data-id=\"rule.no-unsafe\"] details.ex pre').textContent.includes('unsafe')"), "labelled examples sit behind a disclosure");
  check(await cdp.evaluate("!document.body.innerText.includes('null') && !document.body.innerText.includes('undefined')"), "no literal null or undefined is rendered");
  await click('[data-id="rule.no-unsafe"] .e button');
  await cdp.waitFor("document.querySelector('.editor.open form [name=enforcer]')", "the rule editor did not open inline");
  check(await count("dialog[open]") === 0, "editing opens inline, not in a modal");
  check(await cdp.evaluate("document.querySelector('.editor.open [name=enforcer]').value === 'ast' && document.querySelector('.editor.open [name=strength]').value === 'must'"), "the editor starts from the rule's strength and enforcer");
  await fill(".editor.open form", { strength: "should", rationale: "Too strict for internal crates" });
  await cdp.waitFor("document.querySelector('#record-draft')", "the rule review did not render");
  const edit = last((c) => c.workflow === "change" && c.preview === true);
  check(edit.kind === "rule" && edit.record_id === "rule.no-unsafe" && edit.definition.strength === "should", "a rule edit posts its new strength");
  check(edit.definition.enforcer.kind === "ast" && edit.definition.examples.length === 1 && edit.definition.type === "rule", "the enforcer and labelled examples are kept");
  await click("#record-draft");
  await cdp.waitFor("document.querySelector('#announce').textContent.includes('private draft')", "recording did not announce");
  await click("#r-advisory .addrow .btn");
  await cdp.waitFor("document.querySelector('#r-advisory .editor.open form')", "the add-rule editor did not open");
  check(await cdp.evaluate("[...document.querySelectorAll('#r-advisory .editor.open [name=enforcer] option')].map(o => o.value).join() === 'test,lint,formatter,ast,validator,drive'"), "the enforcer picker lists the enforcer kinds");
  await fill("#r-advisory .editor.open form", { content: "Ask a person before deleting data.", enforcer: "validator", value: "scripts/check-deletes", rationale: "Data is irreplaceable" });
  await cdp.waitFor("document.querySelector('#record-draft')", "the new rule review did not render");
  const added = last((c) => c.workflow === "change" && c.preview === true);
  check(added.record_id === "rule.ask-a-person-before-deleting-data" && added.definition.strength === "advisory" && q(added.definition.enforcer) === q({ kind: "validator", command: "scripts/check-deletes" }), "a new rule posts one strength and one enforcer");
  await click('[data-id="principle.prove-it-works"] .e button');
  await cdp.waitFor("document.querySelector('.editor.open form [name=content]')", "the principle editor did not open");
  await fill(".editor.open form", { rationale: "Keep it" });
  await cdp.waitFor("document.querySelector('#record-draft')", "the principle review did not render");
  check(q(last((c) => c.workflow === "change" && c.preview === true).definition) === q({ type: "principle", pstack: "prove-it-works", rationale: null }), "a principle edit keeps its pstack source");
  await click("#migrate-btn");
  await cdp.waitFor("document.querySelector('#announce').textContent.includes('principle draft')", "migration did not announce");
  check(last((c) => c.migrate)?.workflow === "change", "earlier records migrate through wh change --migrate");

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
  check(commands.some((c) => c.review?.verdict === "accept" && c.review.proposal === "proposal.a" && c.preview === false), "accept records the review after preview");

  // Keyboard and theme.
  await cdp.evaluate("document.querySelector('#tab-home').focus(); document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })); true");
  check(await cdp.evaluate("document.activeElement.id === 'tab-changelog'"), "End moves to the last tab");
  await cdp.evaluate("document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })); true");
  check(await cdp.evaluate("document.activeElement.id === 'tab-home'"), "arrow keys wrap around the tabs");
  await click('.theme button[data-theme="dark"]');
  check(await cdp.evaluate("document.documentElement.dataset.theme === 'dark' && localStorage.getItem('wh-theme') === 'dark'"), "the theme toggle applies and remembers dark");
  await click('.theme button[data-theme="light"]');
  check(await cdp.evaluate("document.documentElement.dataset.theme === 'light'"), "the light theme applies");
  await click('.theme button[data-theme="system"]');
  check(await cdp.evaluate("!document.documentElement.hasAttribute('data-theme')"), "system theme follows the OS");

  // Every view at every agreed width.
  for (const width of [320, 390, 768, 1280]) {
    await cdp.viewport(width);
    for (const tab of TABS) {
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
  await cdp.evaluate("(() => { const i = document.querySelector('#cl-q'); i.focus(); i.value = 'abc'; i.dispatchEvent(new Event('input', { bubbles: true })); i.setSelectionRange(0, 3); return true; })()");
  await delay(700);
  check(await cdp.evaluate("(() => { const i = document.querySelector('#cl-q'); return document.activeElement === i && i.value === 'abc' && i.selectionStart === 0 && i.selectionEnd === 3; })()"), "a search re-render keeps the field's focus and selection");

  // Inspection failure: no health claim.
  mode = "error";
  await cdp.viewport(1280);
  await cdp.navigate(`${origin}/`);
  await cdp.waitFor("document.querySelector('#home .empty')", "the error state did not render");
  check(await cdp.evaluate("document.querySelector('#home .empty').textContent.includes('no health claim')"), "an unavailable projection makes no claim");

  const external = cdp.requests.filter((url) => !url.startsWith(origin) && !url.startsWith("data:") && url !== "about:blank");
  check(external.length === 0, `no external requests: ${external.join(", ")}`);
  const errors = cdp.errors.filter((error) => !error.includes("500"));
  check(errors.length === 0, `no page errors: ${errors.join("\n")}`);
  console.log(`PASS synthetic dashboard (${checks} assertions)`);
} finally {
  await browser.close();
  server.close();
}

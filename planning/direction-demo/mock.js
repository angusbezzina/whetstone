// Demo backend: an in-page stand-in for wh dash's service, with fictional
// data. It answers the same requests the product sends and keeps its state in
// memory only. Nothing here runs a check, saves a record or leaves the page.
(() => {
"use strict";
const MODE_KEY = "wh-demo-mode";
let mode = "established";
try { mode = sessionStorage.getItem(MODE_KEY) || "established"; sessionStorage.setItem("whetstone_csrf", "demo"); } catch {}
const label = (tone, text) => ({ tone, label: text });
const now = () => new Date().toISOString().replace(/\.\d+Z$/, "Z");
const OWNER = "Rae Okafor";
const ex = (input, expected, reason) => ({ input, expected, reason, path: null });
const CATALOGUE = [
  ["laziness-protocol", "question", "Bias toward deletion and the smallest change that solves the problem."],
  ["prove-it-works", "mechanical", "Verify against the real artifact (run the feature, read the actual value), not a proxy or 'it compiles'."],
  ["subtract-before-you-add", "question", "Remove dead weight, redundant validators and stub references first, then build on the simpler base."],
  ["boundary-discipline", "question", "Concentrate guards at system boundaries; trust internal types and keep business logic pure."],
  ["type-system-discipline", "mechanical", "Make illegal states unrepresentable and parse external data at boundaries."],
  ["test-behavior-not-implementation", "question", "Call the code the way its users do and assert the result they observe."],
  ["fix-root-causes", "question", "Trace each symptom to its root cause and fix it there."],
  ["never-block-on-the-human", "review", "Proceed, present the result, and let the human course-correct after the fact."],
].map(([id, enforceable, rule]) => ({ id, group: "core", rule, enforceable, suggestion: null }));
const STARTERS = [
  { id: "rule.prove-it-works", title: "Prove it works before done", detected: "pnpm test was detected, so the test suite proves each change at pre-push and in CI.", strength: "must", family: "mechanical", enforcer: "Test", examples: [ex("Changed the invoice parser and ran pnpm test; it passed.", "pass", "The change ran against the real suite."), ex("Changed the invoice parser and reported done.", "flag", "Nothing proved the change works.")] },
  { id: "rule.ask-before-public-api", title: "Ask before changing a public API", detected: "Compares exports, CLI flags and JSON schemas with the last pushed revision at pre-commit, and raises a hand on any change.", strength: "must", family: "mechanical", enforcer: "Public surface", examples: [ex("export function total(lines) {…}\n=== after ===\nexport function total(lines, currency) {…}", "flag", "The exported signature changed.")] },
  { id: "rule.tests-only-when-asked", title: "Only add tests when the task asks", detected: "Asks Jev about each new test file at pre-push, in shadow: answers are recorded, not enforced, until you promote it.", strength: "should", family: "question", enforcer: "Jev question", examples: [ex("commit: Fix typo in README\nadded file test/readme.test.ts", "flag", "A typo fix does not ask for a test.")] },
];
const stats = (x = {}) => ({ checks: 0, commits: 0, flags: 0, accepted: 0, dismissed: 0, undecided: 0, false_flag_rate: null, jev_answers: 0, jev_unavailable: 0, low_confidence: 0, input_tokens: 0, cost_per_check_usd: null, ...x });
const RUNS = { "pre-commit": "pre-commit, pre-push and CI", push: "pre-push and CI", shadow: "pre-push and CI, in shadow (recorded, not enforced)", review: "pre-push and CI, once attested", never: "never; briefed to agents only" };
const FAMILY = { test: ["mechanical", "Test"], lint: ["mechanical", "Lint"], formatter: ["mechanical", "Formatter"], ast: ["mechanical", "AST query"], validator: ["mechanical", "Validator"], drive: ["mechanical", "Drive"], design_tokens: ["mechanical", "Design tokens"], public_surface: ["mechanical", "Public surface"], brief: ["mechanical", "Brief"], question: ["question", "Jev question"], review: ["review", "Review"] };
const cmdText = (e) => e.command || e.query || e.tool || e.feature || e.tokens || e.question || (e.reviewer ? (e.reviewer.by === "person" ? e.reviewer.name : "/interrogate") : "") || (e.surfaces || ["exports", "cli", "json"]).join(", ");
function makeRule(id, statement, strength, enforcer, extra = {}) {
  return { id, statement, strength, enforcer, version: 1, lifecycle: "accepted", pending: null, examples: extra.examples || [], hand_raise: extra.hand_raise || [], paths: extra.paths || [], source: extra.source || "owner", stats: stats(extra.stats), result: extra.result || label("warn", "not run"), last_run: extra.last_run || null, failures: extra.failures || [], changed_at: "2026-09-21T09:00:00Z" };
}
function seed() {
  const S = { established: mode === "established", mission: null, principles: [], rules: [], journal: [], requests: [], flags: [], drafts: 0, last: null, seq: 1 };
  if (!S.established) return S;
  S.mission = { statement: "Invoices that reconcile to the cent, every time.", version: 1 };
  S.principles = [
    { id: "principle.prove-it-works", statement: CATALOGUE[1].rule, source: "pstack prove-it-works (0.15.5)", pstack: "prove-it-works", version: 1 },
    { id: "principle.laziness-protocol", statement: CATALOGUE[0].rule, source: "pstack laziness-protocol (0.15.5)", pstack: "laziness-protocol", version: 1 },
    { id: "principle.custom-money", statement: "Money is integers of the smallest unit, never floats.", source: "the owner's own", pstack: null, version: 1 },
  ];
  S.rules = [
    makeRule("rule.prove-it-works", "Prove it works before calling it done.", "must", { kind: "test", command: "pnpm test" }, { result: label("pass", "pass"), last_run: "2026-09-27T16:40:00Z", stats: { checks: 41, commits: 38 }, source: "starter: pstack:prove-it-works", examples: STARTERS[0].examples }),
    makeRule("rule.ask-before-public-api", "Ask before changing a public API.", "must", { kind: "public_surface" }, { result: label("pass", "pass"), last_run: "2026-09-27T16:40:00Z", hand_raise: ["flag"], stats: { checks: 41, commits: 38, flags: 2, accepted: 2 }, source: "starter: pstack:boundary-discipline", examples: STARTERS[1].examples }),
    makeRule("rule.money-integers", "Money is never stored in a float.", "must", { kind: "ast", query: "(float_literal) @money", language: "typescript" }, { result: label("fail", "fail · 2"), last_run: "2026-09-27T16:40:00Z", paths: ["src/billing/**"], failures: [{ location: "src/billing/tax.ts:41", message: "0.075 is a float in a money path" }, { location: "src/billing/tax.ts:58", message: "amount * 1.2 produces a float" }], stats: { checks: 12, commits: 12, flags: 3, accepted: 3 }, source: "principle: principle.custom-money", examples: [ex("const fee = 250; // cents", "pass", "Integer cents."), ex("const fee = 2.5;", "flag", "A float in a money path.")] }),
    makeRule("rule.design-tokens", "Colours and sizes come from the design tokens.", "should", { kind: "design_tokens", tokens: "src/ui/tokens.css" }, { result: label("pass", "pass"), last_run: "2026-09-27T16:40:00Z", stats: { checks: 30, commits: 30, flags: 9, accepted: 3, dismissed: 5, undecided: 1, false_flag_rate: "62.5%" }, source: "exemplar: github.com/example/ledger-ui" }),
    makeRule("rule.tests-only-when-asked", "Only add tests when the task asks for them.", "should", { kind: "question", question: "Does this change add a test that its commit message does not ask for or explain?", model: "jev-1.13.0", shadow: true }, { result: label("muted", "shadow · 1 flag(s)"), last_run: "2026-09-27T16:40:00Z", stats: { checks: 64, commits: 38, flags: 7, accepted: 6, undecided: 1, jev_answers: 64, input_tokens: 51200, cost_per_check_usd: "0.000410" }, source: "starter: pstack:laziness-protocol", examples: STARTERS[2].examples }),
    makeRule("rule.plain-names", "Name things for the reader, not the author.", "advisory", { kind: "review", reviewer: { by: "interrogate" } }, { result: label("muted", "advisory"), source: "principle: pstack laziness-protocol" }),
  ];
  S.rules[1].pending = { version: 2, title: "Ask before changing a public API or a webhook payload.", proposal: "proposal.webhook" };
  S.drafts = 1;
  S.requests = [
    { id: "verification.hand_3f2", issue: "ledger-7kq", trigger: "must_rule_area", rule: "rule.money-integers", question: "Refunds arrive from the gateway as decimal strings. May I parse them into integer cents at the webhook boundary?", tried: "Read the gateway docs and the refund handler; the existing parser keeps floats.", recommendation: "Parse at the boundary with a strict decimal-to-cents helper and reject more than two places.", raised_at: "2026-09-27T15:12:00Z", status: label("warn", "waiting for the owner"), answer: null, answered_by: null, answered_at: null },
    { id: "verification.hand_1a0", issue: "ledger-5mw", trigger: "vague_spec", rule: null, question: "Should credit notes appear on the monthly statement?", tried: "Searched the spec and past statements.", recommendation: "Show them as negative lines under their invoice.", raised_at: "2026-09-24T10:02:00Z", status: label("pass", "answered"), answer: "Yes, as negative lines under the invoice they credit.", answered_by: OWNER, answered_at: "2026-09-24T11:30:00Z" },
  ];
  S.flags = [
    { receipt: "verification.judgment_91c", rule: "rule.tests-only-when-asked", unit: "test/statement-format.test.ts", at: "2026-09-27T16:40:00Z", jev: true, shadow: true },
    { receipt: "verification.gate_77d", rule: "rule.design-tokens", unit: "src/ui/Receipt.css:12", at: "2026-09-26T11:05:00Z", jev: false, shadow: false },
  ];
  S.last = { at: "2026-09-27T16:40:00Z" };
  const j = (id, at, kind, title, version, status, area, summary, extra = {}) => ({ id, recorded_at: at, kind, title, version, status, owner: kind === "decision" ? OWNER : null, area, summary, note: null, proposal: null, records: [], ...extra });
  S.journal = [
    j("check:1", "2026-09-27T16:40:00Z", "verification", "Checks ran: 4 pass · 1 fail · 0 unknown", null, label("fail", "fail"), "checks", "Rules: rule.money-integers failed at src/billing/tax.ts"),
    j("hand:1", "2026-09-27T15:12:00Z", "decision", "Raised a hand: Refunds arrive from the gateway as decimal strings", null, label("warn", "open"), "requests", "must_rule_area · recommends: parse at the boundary"),
    j("draft:1", "2026-09-26T09:30:00Z", "decision", "Rule revised: Ask before changing a public API or a webhook payload.", "v1 → v2", label("draft", "draft"), "rules", "Webhook payloads are a public contract too.", { proposal: "proposal.webhook" }),
    j("flag:1", "2026-09-25T14:00:00Z", "decision", "Flag dismissed on rule.design-tokens", null, label("muted", "dismissed"), "rules", "The colour is inside an SVG asset, not a stylesheet."),
    j("hand:0", "2026-09-24T11:30:00Z", "decision", "Answered ledger-5mw", null, label("pass", "answered"), "requests", "Yes, as negative lines under the invoice they credit."),
    j("rule:money", "2026-09-22T10:00:00Z", "decision", "Rule added: Money is never stored in a float.", "v1", label("pass", "accepted"), "rules", "Drawn from the principle: money is integers of the smallest unit."),
    j("exemplar:1", "2026-09-21T09:20:00Z", "decision", "Rule added: Colours and sizes come from the design tokens.", "v1", label("pass", "accepted"), "rules", "Drafted from the exemplar github.com/example/ledger-ui (stylelint config)."),
    j("init", "2026-09-21T09:00:00Z", "decision", "Agreement established", "revision 1", label("pass", "accepted"), "mission", "Recorded as one wh init operation: mission, three principles and the three starter rules."),
  ];
  return S;
}
let S = seed();
const entry = (id, kind, title, version, extra = {}) => ({ id, kind, title, detail: [], version, lifecycle: "accepted", pending: null, state: null, owner: OWNER, changed_at: "2026-09-21T09:00:00Z", edit: { kind, record_id: id, content: title, definition: null }, ...extra });
function ruleEntry(r) {
  const [family, enf] = FAMILY[r.enforcer.kind];
  const shadow = r.enforcer.kind === "question" && r.enforcer.shadow !== false;
  const runs = r.strength === "advisory" ? RUNS.never : shadow ? RUNS.shadow : family === "review" ? RUNS.review : ["public_surface", "ast", "lint", "formatter", "design_tokens"].includes(r.enforcer.kind) ? RUNS["pre-commit"] : RUNS.push;
  const definition = { type: "rule", strength: r.strength, enforcer: r.enforcer, examples: r.examples, source: { kind: "owner" }, paths: r.paths, hand_raise: r.hand_raise, privacy: {} };
  return { ...entry(r.id, "rule", r.statement, r.version, { lifecycle: r.lifecycle, pending: r.pending, state: r.result, changed_at: r.changed_at, edit: { kind: "rule", record_id: r.id, content: r.statement, definition } }),
    strength: r.strength, family, enforcer: enf, command: cmdText(r.enforcer), shadow, local_only: false, runs_at: runs, source: r.source, paths: r.paths, examples: r.examples, hand_raise: r.hand_raise, stats: r.stats, unlabelled_flags: S.flags.filter((f) => f.rule === r.id).map(({ receipt, at, unit, shadow }) => ({ receipt, at, unit, commit: null, shadow })), result: r.result };
}
function view() {
  const rules = S.rules.map(ruleEntry);
  const checked = rules.filter((r) => r.strength !== "advisory");
  const gates = checked.map((r) => { const src = S.rules.find((x) => x.id === r.id); const fail = r.result.tone === "fail";
    return { id: r.id, name: r.title, strength: r.strength, family: r.family, mechanism: r.enforcer, command: r.command, eligible: r.lifecycle === "accepted", shadow: r.shadow, result: r.lifecycle === "accepted" ? r.result : label("draft", "draft · not run"), last_run: src.last_run, current: true,
      summary: fail ? null : r.result.tone === "pass" ? "Passed on the current revision." : null, failures: src.failures, recheck: `wh check --rule ${r.id}`,
      brief: fail ? `rule      ${r.title} (v${r.version}, ${r.strength})\ncommand   ${r.command}\nfailures  ${src.failures.map((f) => f.location).join(", ")}\nrepair    within the current task; do not change or weaken the rule\nrecheck   wh check --rule ${r.id}\nstop      if the same finding survives a second repair, raise a hand for ${OWNER}` : null,
      evidence: src.last_run ? [{ kind: "whetstone_evidence", locator: `run/${r.id.replace(".", "_")}.log`, digest: null }] : [], feature: null, team: "private" }; });
  const tally = { pass: gates.filter((g) => g.eligible && g.result.tone === "pass").length, fail: gates.filter((g) => g.eligible && g.result.tone === "fail").length, unknown: gates.filter((g) => g.eligible && g.result.tone === "warn").length };
  const open = S.requests.filter((r) => !r.answer);
  const attention = [];
  for (const r of open) attention.push({ priority: 0, tone: "warn", kind: "raised_hand", kind_label: "raised hand", title: "An agent asks: " + r.question.slice(0, 90) + (r.question.length > 90 ? "…" : ""), text: `Tried: ${r.tried} Recommends: ${r.recommendation}`, actor: OWNER, next: `Answer it: wh change --answer ${r.issue} --content "<your answer>"`, route: "requests", focus: r.id, action_label: "Open the request", agent_instruction: null });
  for (const g of gates.filter((g) => g.eligible && !g.shadow && g.result.tone === "fail")) attention.push({ priority: g.strength === "must" ? 0 : 2, tone: "fail", kind: "agent_repair", kind_label: "agent repair", title: "Failing rule: " + g.name.replace(/\.$/, ""), text: `${g.failures.length} failures in the latest check. The worker that made the change repairs it within task scope, then runs the exact check again.`, actor: "your coding agent", next: "Hand the brief to the agent; return here when it has rerun the check.", route: "checks", focus: g.id, action_label: "Open the failing rule", agent_instruction: g.brief });
  const noisy = S.rules.find((r) => r.id === "rule.design-tokens" && r.strength === "should");
  const suggestions = noisy ? [{ kind: "demotion", rule: noisy.id, reason: `${noisy.stats.dismissed} of ${noisy.stats.accepted + noisy.stats.dismissed} labelled flags were dismissed over ${noisy.stats.commits} commits: more than one false flag per 10 commits. Demote from should to advisory, or reword the rule.`, draft: { strength: "advisory" } }] : [];
  for (const s of suggestions) attention.push({ priority: 2, tone: "muted", kind: "tuning", kind_label: s.kind, title: "Suggested demotion: Colours and sizes come from the design tokens", text: s.reason, actor: OWNER, next: "Record it as a draft with wh change --tune, then accept or withdraw it.", route: "rules", focus: s.rule, action_label: "Open the rule", agent_instruction: null });
  if (S.drafts) attention.push({ priority: 2, tone: "muted", kind: "review", kind_label: "review", title: `${S.drafts} draft${S.drafts === 1 ? "" : "s"} await${S.drafts === 1 ? "s" : ""} your review`, text: "Drafts are private and not in force until you accept them. Nothing has been shared.", actor: OWNER, next: "Accept or withdraw each draft in the changelog.", route: "changelog", focus: null, action_label: "Open the changelog", agent_instruction: null });
  attention.sort((a, b) => a.priority - b.priority);
  const steps = [["tools", "Beads and pstack", "installed and pinned, so records and briefs work", true, true], ["mission", "Mission", "what this project exists to do, in one sentence", !!S.mission, false], ["principles", "Principles", "the pstack principles you hold, or your own", S.principles.length > 0, true], ["exemplars", "Exemplars", "a codebase you admire; rules are drafted from it", S.established, true], ["rules", "Rules", "accept the starter rules and any proposed ones", S.rules.length > 0, false], ["gates", "Gates", "pre-commit and pre-push hooks that run the rules", S.established, false]];
  const journal = S.journal.map((x) => ({ ...x, records: x.records.length ? x.records : [{ record: { id: x.id, record_type: x.kind === "verification" ? "verification_receipt" : "record", record: { summary: x.summary } } }] }));
  return { established: S.established, header: { project: "ledgerline", agreement: S.established ? "owner approved" : "not initialised", visibility: "private", drafts: S.drafts, team: "not shared" },
    onboarding: { steps: steps.map(([key, lbl, hint, done, optional]) => ({ key, label: lbl, hint, done, optional })), missing: S.established ? [] : ["mission", "rules"], command: "wh init", catalogue: CATALOGUE, catalogue_version: "0.15.5", starters: STARTERS.map((s) => ({ ...s, accepted: S.rules.some((r) => r.id === s.id) })), legacy: [], tools: [{ tool: "bd", state: "ok", found: "1.3.0", detail: "Beads 1.1.2 or later is installed.", fix: null }, { tool: "pstack:claude", state: "ok", found: "0.15.5", detail: "pstack is installed for claude.", fix: null }] },
    mission: S.mission ? entry("mission.project", "mission", S.mission.statement, S.mission.version) : null,
    principles: S.principles.map((p) => entry(p.id, "principle", p.statement, p.version, { detail: [{ key: "source", value: p.source, code: false }], edit: { kind: "principle", record_id: p.id, content: p.statement, definition: { type: "principle", pstack: p.pstack, rationale: null } } })),
    rules, features: [],
    checks: { last_complete: S.last ? { at: S.last.at, tally } : null, gates, advisory: rules.filter((r) => r.strength === "advisory").length, shadow: rules.filter((r) => r.shadow).length, receipts: [], driver: { configured: true, path: "whetstone/verify/drive.mjs", label: "verification driver present" } },
    requests: S.requests.map((r) => ({ ...r, answer_command: `wh change --answer ${r.issue} --content "<your answer>"` })), attention,
    latest_change: S.journal[0] ? { title: S.journal[0].title, at: S.journal[0].recorded_at } : null,
    skill: S.established ? { rendered: true, current: true, hosts: ["claude"], rendered_at: "2026-09-21T09:05:00Z", label: label("pass", "current"), projections: [], acknowledged: [] } : { rendered: false, current: false, hosts: [], rendered_at: null, label: label("warn", "not generated"), projections: [], acknowledged: [] },
    hygiene: [], suggestions, _journal: journal };
}
const log = (kind, title, status, area, summary, extra = {}) => S.journal.unshift({ id: "demo:" + S.seq++, recorded_at: now(), kind, title, version: extra.version || null, status, owner: kind === "decision" ? OWNER : null, area, summary, note: null, proposal: extra.proposal || null, records: [] });
const respond = (state, summary, extra = {}) => ({ schema: "whetstone.command-response.v1", state, summary, data: {}, ...extra });
function command(body) {
  if (body.workflow === "init" && body.action === "inspect") return respond("needs_input", "Inspection is complete and read-only.", { expected_revision: 0, resume_token: "demo-0" });
  if (body.workflow === "init" && body.action === "exemplar") return respond("needs_decision", `2 rule drafts were recorded from ${body.exemplar}; accept them in the changelog.`);
  if (body.workflow === "init" && body.action === "agree") {
    const records = [{ id: "mission.project", record_type: "mission", record: { statement: body.mission } }, ...body.principles.map((p) => ({ id: "principle." + p, record_type: "principle", record: { statement: CATALOGUE.find((c) => c.id === p).rule } })), ...body.custom_principles.map((p, i) => ({ id: "principle.custom-" + i, record_type: "principle", record: { statement: p } })), ...body.starters.map((id) => { const s = STARTERS.find((x) => x.id === id); return { id, record_type: "rule", record: { statement: s.title + ".", strength: s.strength, enforcer: { kind: s.enforcer === "Test" ? "test" : s.enforcer === "Public surface" ? "public_surface" : "question" } } }; })];
    if (body.dry_run) return respond("needs_decision", "Dry run: these exact records would be written; nothing was written.", { data: { dry_run: true, records } });
    S.established = true; S.mission = { statement: body.mission, version: 1 };
    S.principles = records.filter((r) => r.record_type === "principle").map((r) => ({ id: r.id, statement: r.record.statement, source: r.id.startsWith("principle.custom") ? "the owner's own" : "pstack " + r.id.slice(10) + " (0.15.5)", pstack: r.id.startsWith("principle.custom") ? null : r.id.slice(10), version: 1 }));
    S.rules = body.starters.map((id) => { const s = STARTERS.find((x) => x.id === id); return makeRule(id, s.title + ".", s.strength, id === "rule.prove-it-works" ? { kind: "test", command: "pnpm test" } : id === "rule.ask-before-public-api" ? { kind: "public_surface" } : { kind: "question", question: "Does this change add a test that its commit message does not ask for or explain?", shadow: true }, { examples: s.examples, source: "starter" }); });
    log("decision", "Agreement established", label("pass", "accepted"), "mission", `Recorded as one wh init operation: mission, ${S.principles.length} principles and ${S.rules.length} rules.`, { version: "revision 1" });
    return respond("needs_input", "The agreement is recorded. Next: install the gates with wh init --action wire --hooks, then wh check.");
  }
  if (body.workflow === "check") {
    for (const r of S.rules.filter((x) => x.strength !== "advisory" && x.lifecycle === "accepted")) { r.last_run = now(); r.stats.checks += 1; if (r.result.tone === "warn") r.result = label("pass", "pass"); }
    S.last = { at: now() };
    const failing = S.rules.filter((r) => r.result.tone === "fail");
    log("verification", `Checks ran: ${S.rules.length - failing.length} pass · ${failing.length} fail · 0 unknown`, label(failing.length ? "fail" : "pass", failing.length ? "fail" : "pass"), "checks", "Rules: " + S.rules.map((r) => r.id).join(", "));
    return respond(failing.some((r) => r.strength === "must") ? "violated" : "success", failing.length ? `A must rule failed.\nFAIL ${failing[0].id}` : "Every rule passed on the current revision.");
  }
  if (body.workflow !== "change") return respond("needs_input", "More input is required.");
  if (body.review) {
    const draft = S.journal.find((x) => x.proposal === body.review.proposal); const rule = S.rules.find((r) => r.pending?.proposal === body.review.proposal || (r.lifecycle === "draft" && r.proposal === body.review.proposal));
    if (body.preview) return respond("needs_decision", "Review the draft.", { data: { preview_only: true, candidates: [{ title: rule?.pending?.title || rule?.statement || draft?.title, version: rule?.pending?.version || 1, replaces: rule?.pending ? rule.version : null }] } });
    if (rule && body.review.verdict === "accept") { if (rule.pending) { rule.statement = rule.pending.title; rule.version = rule.pending.version; if (rule.pending.definition) Object.assign(rule, rule.pending.definition); } rule.lifecycle = "accepted"; }
    if (rule) rule.pending = null;
    if (rule && body.review.verdict === "withdraw" && rule.lifecycle === "draft") S.rules = S.rules.filter((r) => r !== rule);
    if (draft) { draft.proposal = null; draft.status = body.review.verdict === "accept" ? label("pass", "accepted") : label("muted", "withdrawn"); }
    S.drafts = Math.max(0, S.drafts - 1);
    log("decision", (body.review.verdict === "accept" ? "Rule accepted: " : "Draft withdrawn: ") + (rule?.statement || ""), label(body.review.verdict === "accept" ? "pass" : "muted", body.review.verdict === "accept" ? "accepted" : "withdrawn"), "governance", "Explicit solo decision by the owner.");
    return respond("success", body.review.verdict === "accept" ? "The draft was accepted and is now in force for this agreement; nothing was shared." : "The draft was withdrawn; the accepted record stays in force.");
  }
  if (body.flag) {
    const f = S.flags.find((x) => x.receipt === body.flag.receipt); S.flags = S.flags.filter((x) => x !== f);
    const r = S.rules.find((x) => x.id === f?.rule); if (r) { r.stats.undecided = Math.max(0, r.stats.undecided - 1); r.stats[body.flag.verdict === "accept" ? "accepted" : "dismissed"] += 1; const n = r.stats.accepted + r.stats.dismissed; r.stats.false_flag_rate = n ? (100 * r.stats.dismissed / n).toFixed(1) + "%" : null; }
    const verdict = body.flag.verdict === "accept" ? "accepted" : "dismissed";
    log("decision", `Flag ${verdict} on ${f?.rule}`, label(verdict === "accepted" ? "pass" : "muted", verdict), "rules", body.rationale);
    return respond("success", `The flag on ${f?.rule} was ${verdict}; it counts toward the rule's false-flag rate.`);
  }
  if (body.answer) {
    const r = S.requests.find((x) => x.issue === body.answer); Object.assign(r, { answer: body.content, answered_by: OWNER, answered_at: now(), status: label("pass", "answered") });
    log("decision", "Answered " + r.issue, label("pass", "answered"), "requests", body.content);
    return respond("success", `${r.issue} was answered and closed in Beads; the answer is in the trail.`);
  }
  if (body.tune) {
    const r = S.rules.find((x) => x.id === "rule.design-tokens"); if (r && !r.pending) { r.pending = { version: r.version + 1, title: r.statement, proposal: "proposal.tune", definition: { strength: "advisory" } }; S.drafts += 1; log("decision", "Rule demoted: " + r.statement, label("draft", "draft"), "rules", "Proposed by the rule's record of flags.", { proposal: "proposal.tune", version: `v${r.version} → v${r.version + 1}` }); }
    return respond("success", "Recorded 1 tuning draft; accept or withdraw it in the changelog.");
  }
  if (body.migrate) return respond("success", "Nothing to migrate.");
  if (body.expected_revision == null) return respond("needs_input", "More owner input is required; no agreement change was recorded.", { expected_revision: 1, resume_token: "demo-1" });
  const existing = S.rules.find((r) => r.id === body.record_id);
  const before = body.kind === "rule" && existing ? { statement: existing.statement, strength: existing.strength, enforcer: existing.enforcer } : body.kind === "mission" && S.mission ? { statement: S.mission.statement } : null;
  const after = body.kind === "rule" ? { statement: body.content, strength: body.definition.strength, enforcer: body.definition.enforcer } : { statement: body.content };
  if (body.preview) return respond("needs_decision", "Review the exact draft.", { data: { preview_only: true, base_revision: existing?.version || 0, diff: { before: before && { record: before }, after: { record: after } } } });
  const proposal = "proposal.demo" + S.seq;
  if (body.kind === "rule") {
    if (existing) existing.pending = { version: existing.version + 1, title: body.content, proposal, definition: { strength: body.definition.strength, enforcer: body.definition.enforcer } };
    else { const r = makeRule(body.record_id, body.content, body.definition.strength, body.definition.enforcer, { result: label("draft", "draft · not run") }); r.lifecycle = "draft"; r.proposal = proposal; S.rules.push(r); }
  }
  S.drafts += 1;
  log("decision", (existing ? "Rule revised: " : "Rule added: ") + body.content, label("draft", "draft"), body.kind === "rule" ? "rules" : body.kind, body.rationale, { proposal, version: existing ? `v${existing.version} → v${existing.version + 1}` : "v1" });
  return respond("success", "The private draft was recorded. It is not in force until you accept it; nothing was shared.");
}
const json = (body, status = 200) => Promise.resolve(new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } }));
window.fetch = (url, options = {}) => {
  const path = String(url);
  if (path === "/session/edit") return json({ mode: "edit" });
  if (path === "/api/inspect") { const query = options.body ? JSON.parse(options.body) : {}; const v = view(); const needle = (query.search || "").toLowerCase(); const changelog = v._journal.filter((x) => (!needle || [x.title, x.summary, x.area].join(" ").toLowerCase().includes(needle)) && (!query.as_of || x.recorded_at <= query.as_of)); delete v._journal; return json({ schema: "whetstone.command-response.v1", state: "success", summary: "Read-only project shape and decision history are ready.", data: { current: v, changelog } }); }
  if (path === "/api/command") return new Promise((done) => setTimeout(() => done(json(command(JSON.parse(options.body)))), JSON.parse(options.body).workflow === "check" ? 900 : 120));
  return json({ state: "unknown", summary: "Not part of the demo." }, 404);
};
// Links the product serves (trail export, evidence) have no server here.
document.addEventListener("click", (event) => { const link = event.target.closest?.('a[href^="/api/"]'); if (link) { event.preventDefault(); const a = document.getElementById("announce"); if (a) a.textContent = "The demo has no server; wh dash serves " + link.getAttribute("href") + "."; } });
document.addEventListener("DOMContentLoaded", () => {
  for (const input of document.querySelectorAll("input[name=demo]")) { input.checked = input.value === mode; input.addEventListener("change", () => { try { sessionStorage.setItem(MODE_KEY, input.value); } catch {} location.reload(); }); }
  document.getElementById("demo-reset")?.addEventListener("click", () => location.reload());
});
})();

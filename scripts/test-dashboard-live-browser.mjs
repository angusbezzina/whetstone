#!/usr/bin/env node
// Live browser proof against the real dashboard service and private store:
// agree the first rules through onboarding, draft and accept a rule from
// Rules, run the checks, label the flag it raises, answer a raised hand, and
// prove everything persists across a reload at every agreed width.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { delay, launch } from "./cdp.mjs";

const url = process.env.WH_DASHBOARD_URL;
const root = process.env.WH_PROJECT_ROOT;
assert.ok(url, "WH_DASHBOARD_URL is required");
assert.ok(root, "WH_PROJECT_ROOT is required");
const origin = new URL(url).origin;
// Optional: the whetstone binary (raises a hand as an agent would) and a
// directory for screenshots of every view at every width.
const bin = process.env.WH_BIN;
const shots = process.env.WH_SCREENSHOTS;

const browser = await launch();
if (!browser) {
  console.log("SKIP live dashboard browser test: Chrome/Chromium is unavailable");
  process.exit(77);
}
const { cdp } = browser;
const q = JSON.stringify;
const click = (selector) => cdp.evaluate(`(() => { const n = document.querySelector(${q(selector)}); if (!n) throw new Error("missing " + ${q(selector)}); n.click(); return true; })()`);
const fill = (form, values, submit = true) => cdp.evaluate(`(() => { const f = document.querySelector(${q(form)}); if (!f) throw new Error("missing " + ${q(form)}); for (const [k, v] of Object.entries(${q(values)})) { const n = f.querySelector('[name="' + k + '"]'); n.value = v; n.dispatchEvent(new Event("change")); } ${submit ? "f.requestSubmit();" : ""} return true; })()`);
const text = (selector) => cdp.evaluate(`document.querySelector(${q(selector)})?.textContent ?? null`);
let checks = 0;
const check = (condition, message) => { assert.ok(condition, message); checks += 1; };
const TABS = ["home", "rules", "checks", "changelog", "requests"];
async function shot(name) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  const { contentSize } = await cdp.send("Page.getLayoutMetrics");
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true, clip: { x: 0, y: 0, width: contentSize.width, height: Math.min(contentSize.height, 6000), scale: 1 } });
  writeFileSync(join(shots, `${name}.png`), Buffer.from(data, "base64"));
}

try {
  await cdp.navigate(url);
  await cdp.waitFor("document.querySelector('#onboard h1')", "the fresh project did not show onboarding");
  check(await cdp.evaluate("[...document.querySelectorAll('#onboard .steps li')].map(l => l.dataset.step).join() === 'tools,mission,principles,exemplars,rules,gates'"), "onboarding lists the six steps in order");
  check(await cdp.evaluate("['checks','changelog','requests'].every(t => document.querySelector('#tab-' + t).getAttribute('aria-disabled') === 'true')"), "checks, changelog and requests are gated before the agreement");
  check(await cdp.evaluate("!/eight|core value|metric/i.test(document.body.innerText)"), "no removed decision, value or metric language");
  for (const width of [320, 1280]) {
    await cdp.viewport(width);
    const overflow = await cdp.evaluate("document.documentElement.scrollWidth - window.innerWidth");
    check(overflow <= 1, `fresh onboarding fits at ${width}px (overflow ${overflow})`);
    await shot(`fresh-${width}`);
  }

  // Onboarding: mission, catalogue principles, a custom principle, starters.
  await click("#start-onboarding");
  await cdp.waitFor("document.querySelector('#init-form #ob-principles input')", "the onboarding form did not open");
  check(await cdp.evaluate("document.querySelectorAll('#ob-principles input[name=principle]').length >= 10"), "the pstack principle catalogue is offered");
  check(await cdp.evaluate("document.querySelectorAll('#ob-starters input[name=starter]:checked').length === 3"), "the three starter rules are offered and preselected");
  await shot("onboarding-form-1280");
  await cdp.evaluate(`(() => { for (const id of ["prove-it-works", "laziness-protocol"]) document.querySelector('#ob-principles input[value="' + id + '"]').checked = true; return true; })()`);
  await fill("#init-form", { mission: "Keep project intent inspectable.", custom_principles: "Prefer boring code." });
  await cdp.waitFor("document.querySelector('#confirm-init')", "the exact agreement review did not render", 20_000);
  check(await cdp.evaluate("document.querySelectorAll('#onboard .review .row').length === 7"), "the review lists mission, three principles and three rules");
  await click("#confirm-init");
  await cdp.waitFor("document.querySelector('#mission-line')?.textContent === 'Keep project intent inspectable.'", "the agreement did not establish the dashboard", 45_000);
  check(await cdp.evaluate("document.querySelector('#tab-checks').getAttribute('aria-disabled') === 'false'"), "checks open after the agreement");
  check(await cdp.evaluate("document.querySelectorAll('#home-rules .row').length === 3"), "home lists the three rules in force");
  check(await cdp.evaluate("document.querySelectorAll('#attention-primary .btn.primary').length === 1 && document.querySelectorAll('#home .btn.primary').length === 1"), "home has exactly one primary action");

  // Rules: grouped by strength, one enforcer, shadow status, examples.
  await click("#tab-rules");
  await cdp.waitFor("document.querySelector('#r-must .rec')", "rules did not render");
  check(await cdp.evaluate("[...document.querySelectorAll('#rules .panel .ph h2')].map(h => h.firstChild.textContent).slice(0, 5).join('|') === 'Mission|Principles|Must|Should|Advisory'"), "rules are grouped mission, principles, then by strength");
  check(await cdp.evaluate("document.querySelectorAll('#r-principles .rec').length === 3"), "three principles are recorded");
  check(await cdp.evaluate("document.querySelectorAll('#r-must .rec').length === 2 && document.querySelectorAll('#r-should .rec').length === 1"), "two must rules and one should rule");
  check(await cdp.evaluate("/shadow/.test(document.querySelector('[data-id=\"rule.tests-only-when-asked\"] .badge')?.textContent)"), "the Jev starter shows its shadow status");
  check(await cdp.evaluate("/question/.test(document.querySelector('[data-id=\"rule.tests-only-when-asked\"] .fam').textContent) && /pre-push/.test(document.querySelector('[data-id=\"rule.tests-only-when-asked\"]').textContent)"), "a rule names its enforcer family and where it runs");
  check(await cdp.evaluate("document.querySelector('[data-id=\"rule.ask-before-public-api\"] details.ex li') !== null"), "labelled examples sit behind a disclosure");

  // Draft a should rule whose check fails, then accept it in the changelog.
  await click("#r-should .addrow .btn");
  await cdp.waitFor("document.querySelector('#r-should .editor.open form')", "the add-rule editor did not open");
  await fill("#r-should .editor.open form", { content: "No TODO comments.", enforcer: "test", value: "false", rationale: "They rot." });
  await cdp.waitFor("document.querySelector('#record-draft')", "the rule review did not render", 20_000);
  check(await cdp.evaluate("document.querySelector('.review .diff .after').textContent.includes('No TODO comments.')"), "the review shows the new rule");
  await click("#record-draft");
  await cdp.waitFor("document.querySelector('#draft-count').textContent === '1'", "the draft was not recorded", 45_000);
  check(await cdp.evaluate("document.querySelector('[data-id=\"rule.no-todo-comments\"]')?.classList.contains('draft')"), "the new rule shows as a draft");
  await click("#tab-changelog");
  await cdp.waitFor("[...document.querySelectorAll('.entry .act .btn')].some(b => b.textContent === 'Accept')", "the draft is not reviewable in the changelog", 45_000);
  await cdp.evaluate("[...document.querySelectorAll('.entry .act .btn')].find(b => b.textContent === 'Accept').click(); true");
  await cdp.waitFor("document.querySelector('#confirm-review')", "the acceptance review did not render");
  await click("#confirm-review");
  await cdp.waitFor("document.querySelector('#draft-count').textContent === '0'", "the draft was not accepted", 45_000);

  // Run the checks for real; the should rule fails and raises a flag.
  await click("#tab-checks");
  await cdp.waitFor("document.querySelector('#run-btn')", "checks did not render");
  await click("#run-btn");
  await cdp.waitFor("document.querySelector('#gate-rule\\\\.no-todo-comments .state')?.classList.contains('fail')", "the failing rule did not fail", 60_000);
  check(await cdp.evaluate("document.querySelector('#gate-rule\\\\.ask-before-public-api .state').textContent === 'pass'"), "the public-surface rule passes");
  check(await cdp.evaluate("document.querySelector('#det-rule\\\\.no-todo-comments pre.brief') !== null"), "the failing rule carries a repair brief");
  check(
    await cdp.evaluate(`(async () => {
      const link = document.querySelector('#det-rule\\\\.no-todo-comments .evid a');
      if (!link) return false;
      const response = await fetch(link.getAttribute('href'));
      return response.ok && (await response.text()).length > 0;
    })()`),
    "the failing rule's evidence opens from Checks",
  );
  check(await cdp.evaluate("document.querySelectorAll('#flags .row.flag').length === 1"), "the failure is listed as a flag to label");
  await click("#flags .row.flag .btn.quiet");
  await cdp.waitFor("document.querySelector('#flags .editor.open textarea')", "the flag label form did not open");
  await fill("#flags .editor.open form", { rationale: "The command is a placeholder, not a real check." });
  await cdp.waitFor("document.querySelectorAll('#flags .row.flag').length === 0", "the flag was not labelled", 45_000);
  await click("#tab-rules");
  await cdp.waitFor("/1 false/.test(document.querySelector('[data-id=\"rule.no-todo-comments\"] .stats')?.textContent || '')", "the dismissed flag did not reach the rule's record");

  // A raised hand, filed as an agent would, answered from Requests.
  let answered = false;
  if (bin) {
    const raised = spawnSync(bin, ["check", "--raise-hand", "--project-dir", root, "--question", "Should the parser accept tabs?", "--tried", "Read the spec", "--recommend", "Treat tabs as spaces", "--trigger", "vague-spec", "--json"], { encoding: "utf8" });
    const response = JSON.parse(raised.stdout || "{}");
    if (response.state === "needs_decision") {
      await cdp.navigate(`${origin}/`);
      await cdp.waitFor("document.querySelector('#req-count')?.textContent === '1'", "the open request is not counted");
      await click("#tab-requests");
      await cdp.waitFor("document.querySelector('#requests .req:not(.done) form')", "the request did not render");
      check(await cdp.evaluate("/Treat tabs as spaces/.test(document.querySelector('#requests .req').textContent)"), "the request shows the agent's recommendation");
      check(await cdp.evaluate("/read only|Editing is not available/.test(document.body.innerText) === false"), "requests are answerable in an edit session");
      await fill("#requests .req form", { content: "Yes, tabs are four spaces." });
      await cdp.waitFor("document.querySelector('#requests .req.done')", "the answer was not recorded", 45_000);
      answered = true;
    } else {
      console.log(`note: no hand raised (${response.summary || raised.stderr.trim()}); skipping the Requests answer`);
    }
  }

  // Persistence and a read of the same records after reload.
  await cdp.navigate(`${origin}/`);
  await cdp.waitFor("document.querySelector('#mission-line')", "the dashboard did not reload");
  await click("#tab-rules");
  await cdp.waitFor("document.querySelector('[data-id=\"rule.no-todo-comments\"]')", "rules did not reload");
  check(await cdp.evaluate("!document.querySelector('[data-id=\"rule.no-todo-comments\"]').classList.contains('draft')"), "the accepted rule persists");
  await click("#tab-checks");
  await cdp.waitFor("document.querySelector('#gate-rule\\\\.no-todo-comments')", "checks did not reload");
  check(await cdp.evaluate("document.querySelector('#gate-rule\\\\.no-todo-comments .state').classList.contains('fail') && document.querySelectorAll('#flags .row.flag').length === 0"), "the result and the flag label persist");
  if (answered) {
    await click("#tab-requests");
    check(await cdp.evaluate("/tabs are four spaces/.test(document.querySelector('#requests .req.done').textContent)"), "the answer persists");
  }

  for (const width of [320, 390, 768, 1280]) {
    await cdp.viewport(width);
    for (const tab of TABS) {
      await click(`#tab-${tab}`);
      await delay(80);
      const overflow = await cdp.evaluate("document.documentElement.scrollWidth - window.innerWidth");
      check(overflow <= 1, `${tab} fits at ${width}px with real data (overflow ${overflow})`);
      await shot(`${tab}-${width}`);
    }
  }
  const external = cdp.requests.filter((request) => !request.startsWith(origin) && !request.startsWith("data:") && request !== "about:blank");
  check(external.length === 0, `no external requests: ${external.join(", ")}`);
  check(cdp.errors.length === 0, `no page errors: ${cdp.errors.join("\n")}`);
  console.log(`PASS live dashboard (${checks} assertions${answered ? ", request answered" : ""})`);
} finally {
  await browser.close();
}

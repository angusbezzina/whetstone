#!/usr/bin/env node
// Live browser proof against the real dashboard service and private store:
// establish foundations from the UI, draft and accept a value, run the first
// gate, and prove everything persists across a reload.

import assert from "node:assert/strict";
import { delay, launch } from "./cdp.mjs";

const url = process.env.WH_DASHBOARD_URL;
assert.ok(url, "WH_DASHBOARD_URL is required");
assert.ok(process.env.WH_PROJECT_ROOT, "WH_PROJECT_ROOT is required");
const origin = new URL(url).origin;

const browser = await launch();
if (!browser) {
  console.log("SKIP live dashboard browser test: Chrome/Chromium is unavailable");
  process.exit(77);
}
const { cdp } = browser;
const click = (selector) => cdp.evaluate(`(() => { const n = document.querySelector(${JSON.stringify(selector)}); if (!n) throw new Error("missing " + ${JSON.stringify(selector)}); n.click(); return true; })()`);
const fill = (form, values) => cdp.evaluate(`(() => { const f = document.querySelector(${JSON.stringify(form)}); for (const [k, v] of Object.entries(${JSON.stringify(values)})) f.querySelector('[name="' + k + '"]').value = v; f.requestSubmit(); return true; })()`);
let checks = 0;
const check = (condition, message) => { assert.ok(condition, message); checks += 1; };

try {
  await cdp.navigate(url);
  await cdp.waitFor("document.querySelector('.onboard h1')", "the fresh project did not show onboarding");
  check(await cdp.evaluate("document.querySelector('#tab-checks').getAttribute('aria-disabled') === 'true'"), "checks are gated before init");
  await click(".onboard .btn.primary");
  await cdp.waitFor("document.querySelector('#init-form')", "the first-agreement form did not open");
  await fill("#init-form", {
    mission: "Keep project intent inspectable.",
    desired_outcome: "Routine drift is repaired before handoff.",
    values: "Evidence before assertion.",
    philosophy: "Typed services own deterministic work.",
    owner: "Owner",
    revision_triggers: "a gate fails twice in a week",
    initial_safeguard: "The toolchain answers.",
    safeguard_scope: "repository",
    gate_command: "git --version",
  });
  await cdp.waitFor("document.querySelector('#confirm-init')", "the exact agreement review did not render");
  check(await cdp.evaluate("document.querySelectorAll('.onboard.review .row').length === 5"), "the review lists the five exact records");
  await click("#confirm-init");
  await cdp.waitFor("document.querySelector('#mission-line')?.textContent === 'Keep project intent inspectable.'", "the agreement did not establish the dashboard", 45_000);
  check(await cdp.evaluate("document.querySelector('#tab-checks').getAttribute('aria-disabled') === 'false'"), "checks open after init");

  // Draft a new value inline, then accept it from the changelog.
  await click("#tab-foundations");
  await cdp.waitFor("document.querySelector('#st-values .addrow .btn')", "the values stage did not render");
  await click("#st-values .addrow .btn");
  await cdp.waitFor("document.querySelector('#st-values .editor.open form')", "the add-value editor did not open");
  await fill("#st-values .editor.open form", { content: "Small safe steps.", rationale: "We ship in increments." });
  await cdp.waitFor("document.querySelector('#record-draft')", "the value review did not render");
  check(await cdp.evaluate("document.querySelector('.review .diff .after').textContent.includes('Small safe steps.')"), "the review shows the new value");
  await click("#record-draft");
  await cdp.waitFor("document.querySelector('#draft-count').textContent === '1'", "the draft was not recorded", 45_000);
  check(await cdp.evaluate("[...document.querySelectorAll('#st-values .rec')].some(r => r.classList.contains('draft') && r.textContent.includes('Small safe steps.'))"), "the new value shows as a draft");
  await click("#tab-changelog");
  await cdp.waitFor("[...document.querySelectorAll('.entry .act .btn')].some(b => b.textContent === 'Accept')", "the draft is not reviewable in the changelog", 45_000);
  await cdp.evaluate("[...document.querySelectorAll('.entry .act .btn')].find(b => b.textContent === 'Accept').click(); true");
  await cdp.waitFor("document.querySelector('#confirm-review')", "the acceptance review did not render");
  await click("#confirm-review");
  await cdp.waitFor("document.querySelector('#draft-count').textContent === '0'", "the draft was not accepted", 45_000);

  // Run the first gate for real.
  await click("#tab-checks");
  await cdp.waitFor("document.querySelector('#run-btn')", "checks did not render");
  await click("#run-btn");
  await cdp.waitFor("document.querySelector('#gate-standard\\\\.initial-gate .state')?.textContent === 'pass'", "the first gate did not pass", 60_000);

  // Persistence and a read of the same records after reload.
  await cdp.navigate(`${origin}/`);
  await cdp.waitFor("document.querySelector('#mission-line')", "the dashboard did not reload");
  await click("#tab-foundations");
  await cdp.waitFor("document.querySelector('#st-values')", "foundations did not reload");
  check(await cdp.evaluate("[...document.querySelectorAll('#st-values .rec')].some(r => !r.classList.contains('draft') && r.textContent.includes('Small safe steps.'))"), "the accepted value persists");
  await click("#tab-checks");
  await cdp.waitFor("document.querySelector('#gate-standard\\\\.initial-gate')", "checks did not reload");
  check(await cdp.evaluate("document.querySelector('#gate-standard\\\\.initial-gate .state').textContent === 'pass'"), "the gate result persists");
  check(
    await cdp.evaluate(`(async () => {
      const link = document.querySelector('#det-standard\\\\.initial-gate .evid a');
      if (!link) return false;
      const response = await fetch(link.getAttribute('href'));
      return response.ok && (await response.text()).length > 0;
    })()`),
    "the gate's evidence opens from Checks",
  );

  for (const width of [320, 390, 768, 1024]) {
    await cdp.viewport(width);
    for (const tab of ["dashboard", "foundations", "checks", "changelog"]) {
      await click(`#tab-${tab}`);
      await delay(80);
      const overflow = await cdp.evaluate("document.documentElement.scrollWidth - window.innerWidth");
      check(overflow <= 1, `${tab} fits at ${width}px with real data (overflow ${overflow})`);
    }
  }
  const external = cdp.requests.filter((request) => !request.startsWith(origin) && !request.startsWith("data:") && request !== "about:blank");
  check(external.length === 0, `no external requests: ${external.join(", ")}`);
  check(cdp.errors.length === 0, `no page errors: ${cdp.errors.join("\n")}`);
  console.log(`PASS live dashboard (${checks} assertions)`);
} finally {
  await browser.close();
}

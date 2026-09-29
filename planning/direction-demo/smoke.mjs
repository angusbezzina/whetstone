#!/usr/bin/env node
// Dependency-free smoke test for the dashboard direction reference.
// Drives the static HTML in headless Chrome over the DevTools protocol,
// asserts the behaviours the reference is meant to demonstrate, and writes
// screenshots to a temporary directory. Nothing here touches the product.
import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawn } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const target = pathToFileURL(join(here, "index.html")).href;
const chrome = [
  process.env.CHROME_BIN,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].filter(Boolean).find(existsSync);
if (!chrome) {
  console.log("SKIP direction-demo smoke: Chrome/Chromium is unavailable");
  process.exit(77);
}
const output = mkdtempSync(join(tmpdir(), "whetstone-direction-"));
const profile = mkdtempSync(join(tmpdir(), "whetstone-direction-profile-"));
const browser = spawn(chrome, [
  "--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "--disable-gpu", "--hide-scrollbars",
  "--allow-file-access-from-files", "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function waitForFile(path) {
  for (let i = 0; i < 400; i += 1) {
    if (existsSync(path)) return readFileSync(path, "utf8");
    await delay(25);
  }
  throw new Error(`Timed out waiting for ${path}`);
}
class Cdp {
  constructor(url) { this.socket = new WebSocket(url); this.sequence = 0; this.pending = new Map(); this.errors = []; }
  open() {
    return new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", reject, { once: true });
      this.socket.addEventListener("message", ({ data }) => {
        const message = JSON.parse(data);
        if (message.id && this.pending.has(message.id)) {
          const entry = this.pending.get(message.id); this.pending.delete(message.id);
          message.error ? entry.reject(new Error(JSON.stringify(message.error))) : entry.resolve(message.result);
        } else if (message.method === "Runtime.exceptionThrown" || (message.method === "Runtime.consoleAPICalled" && message.params.type === "error")) {
          this.errors.push(JSON.stringify(message.params).slice(0, 300));
        }
      });
    });
  }
  send(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => { this.pending.set(id, { resolve, reject }); this.socket.send(JSON.stringify({ id, method, params })); });
  }
  async evaluate(expression) {
    const result = await this.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails).slice(0, 400));
    return result.result.value;
  }
}
let assertions = 0;
const check = (condition, message) => { assert.ok(condition, message); assertions += 1; };
try {
  const port = (await waitForFile(join(profile, "DevToolsActivePort"))).split("\n")[0];
  let page;
  for (let i = 0; i < 400 && !page; i += 1) {
    try {
      const pages = await fetch(`http://127.0.0.1:${port}/json/list`).then((r) => r.json());
      page = pages.find((c) => c.type === "page" && !c.url.startsWith("chrome-extension:"));
    } catch {}
    if (!page) await delay(25);
  }
  assert.ok(page, "Chrome did not expose a page target");
  const cdp = new Cdp(page.webSocketDebuggerUrl);
  await cdp.open(); await cdp.send("Page.enable"); await cdp.send("Runtime.enable");
  await cdp.send("Page.navigate", { url: target }); await delay(600);
  const shot = async (name, width) => {
    const { contentSize } = await cdp.send("Page.getLayoutMetrics");
    const result = await cdp.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true, clip: { x: 0, y: 0, width, height: Math.min(contentSize.height, 5000), scale: 1 } });
    writeFileSync(join(output, `${name}.png`), Buffer.from(result.data, "base64"));
  };
  const open = (tab) => cdp.evaluate(`document.querySelector("#tab-${tab}").click(); true`);
  const until = async (expression, message, timeout = 4000) => { for (let t = 0; t < timeout; t += 50) { if (await cdp.evaluate(expression)) return; await delay(50); } throw new Error(message); };
  // Switching reloads the page; wait for the new page's first render, not a fixed time.
  const demo = async (state) => { await cdp.evaluate(`document.querySelector('input[name=demo][value=${state}]').click(); true`); await delay(300); await until(`document.readyState === 'complete' && document.querySelector('input[name=demo][value=${state}]')?.checked && !!document.querySelector(${state === 'fresh' ? "'#onboard h1'" : "'#mission-line'"})`, `the ${state} demo did not render`, 8000); };
  const fill = (form, values) => cdp.evaluate(`(() => { const f = document.querySelector(${JSON.stringify(form)}); for (const [k, v] of Object.entries(${JSON.stringify(values)})) { const n = f.querySelector('[name="' + k + '"]'); n.value = v; n.dispatchEvent(new Event("change")); } f.requestSubmit(); return true; })()`);

  // Fresh project: onboarding only, later views gated.
  await demo("fresh");
  check(await cdp.evaluate(`!!document.querySelector('#onboard h1') && document.querySelectorAll('#home .panel').length === 0`), "fresh home holds the onboarding only");
  check(await cdp.evaluate(`[...document.querySelectorAll('#onboard .steps li')].map(l => l.dataset.step).join() === 'tools,mission,principles,exemplars,rules,gates'`), "onboarding lists tools, mission, principles, exemplars, rules and gates");
  check(await cdp.evaluate(`['checks','changelog','requests'].every(t => document.querySelector('#tab-' + t).getAttribute('aria-disabled') === 'true')`), "checks, changelog and requests are gated before init");
  check(await cdp.evaluate(`document.querySelector('#agreement-state').textContent === 'not initialised'`), "top bar says not initialised");
  check(await cdp.evaluate(`!/eight decisions|core values|key metrics|foundations/i.test(document.body.innerText)`), "no removed decision, value, metric or foundations language");
  await open("checks"); check(await cdp.evaluate(`document.querySelector('#home').hidden === false`), "a gated view is not opened");
  await shot("onboarding", 1280);
  await cdp.evaluate(`document.querySelector('#start-onboarding').click(); true`);
  await until(`document.querySelector('#init-form')`, "the onboarding form did not open");
  check(await cdp.evaluate(`document.querySelectorAll('#ob-principles input[name=principle]').length === 8 && document.querySelectorAll('#ob-starters input:checked').length === 3`), "the pstack catalogue and three starter rules are offered");
  await cdp.evaluate(`document.querySelector('#ob-principles input[value="prove-it-works"]').checked = true; true`);
  await shot("onboarding-form", 1280);
  for (const width of [320, 390]) {
    await cdp.send("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: true });
    await delay(80);
    const overflow = await cdp.evaluate(`document.documentElement.scrollWidth - window.innerWidth`);
    check(overflow <= 1, `the onboarding form fits at ${width}px (overflow ${overflow}px)`);
    await shot(`onboarding-form-${width}`, width);
  }
  await cdp.send("Emulation.clearDeviceMetricsOverride");
  await fill("#init-form", { mission: "Invoices that reconcile to the cent.", custom_principles: "Money is integers." });
  await until(`document.querySelector('#confirm-init')`, "the agreement review did not render");
  check(await cdp.evaluate(`document.querySelectorAll('#onboard .review .row').length === 6`), "the review lists mission, two principles and three rules");
  await cdp.evaluate(`document.querySelector('#confirm-init').click(); true`);
  await until(`document.querySelector('#mission-line')?.textContent === 'Invoices that reconcile to the cent.'`, "the agreement did not establish the project");
  check(await cdp.evaluate(`document.querySelector('#tab-checks').getAttribute('aria-disabled') === 'false'`), "checks open once agreed");

  // Established project: one primary action, honest states, rules by strength.
  await demo("established");
  check(await cdp.evaluate(`document.querySelectorAll('#home .btn.primary').length === 1`), "home has exactly one primary action");
  check(await cdp.evaluate(`/An agent asks/.test(document.querySelector('#attention-primary h3').textContent)`), "a raised hand leads the attention list");
  check(await cdp.evaluate(`[...document.querySelectorAll('#home .state.pass')].every(s => !/unknown|not|shadow|draft|stale/.test(s.textContent))`), "unknown, shadow and draft states are never green");
  check(await cdp.evaluate(`document.querySelector('#req-count').textContent === '1'`), "the open request is counted on its tab");
  await open("rules"); await delay(150);
  check(await cdp.evaluate(`[...document.querySelectorAll('#rules .panel .ph h2')].map(h => h.firstChild.textContent).join('|') === 'Mission|Principles|Must|Should|Advisory|Suggestions|Features'`), "rules are grouped mission, principles, must, should, advisory, then suggestions");
  check(await cdp.evaluate(`document.querySelector('[data-id="rule.tests-only-when-asked"] .badge').textContent === 'shadow'`), "the Jev rule shows its shadow status");
  check(await cdp.evaluate(`/false-flag rate 62.5%/.test(document.querySelector('[data-id="rule.design-tokens"] .stats').textContent)`), "a rule shows its false-flag rate");
  check(await cdp.evaluate(`/mechanical/.test(document.querySelector('[data-id="rule.money-integers"] .fam').textContent) && /pre-commit/.test(document.querySelector('[data-id="rule.money-integers"]').textContent)`), "a rule names its one enforcer family and where it runs");
  check(await cdp.evaluate(`/draft v2 pending/.test(document.querySelector('[data-id="rule.ask-before-public-api"] .ver').textContent)`), "a pending draft is labelled beside the rule in force");

  // Edit → review → draft keeps the accepted rule in force.
  await cdp.evaluate(`document.querySelector('[data-id="rule.money-integers"] .e button').click(); true`); await delay(300);
  check(await cdp.evaluate(`document.querySelector('.editor.open form [name=enforcer]').value === 'ast'`), "edit opens inline with the rule's enforcer");
  await fill(".editor.open form", { content: "Money is never a float, in any path.", rationale: "Tax code escaped the path glob" });
  await until(`document.querySelector('#record-draft')`, "the review did not render");
  check(await cdp.evaluate(`document.querySelector('.review .diff .after').textContent.includes('in any path') && /shared/.test(document.querySelector('.review .effects').textContent)`), "review shows before/after and effects");
  await shot("review", 1280);
  await cdp.evaluate(`document.querySelector('#record-draft').click(); true`);
  await until(`document.querySelector('#draft-count').textContent === '2'`, "the draft count did not increment");
  check(await cdp.evaluate(`document.querySelector('[data-id="rule.money-integers"] .c').textContent === 'Money is never stored in a float.'`), "accepted content stays in force");

  // Checks: failure, brief, run, flags.
  await open("checks"); await delay(150);
  check(await cdp.evaluate(`document.querySelector('#gate-rule\\\\.money-integers .state').classList.contains('fail') && document.querySelector('#det-rule\\\\.money-integers pre.brief') !== null`), "a failing rule carries its repair brief");
  check(await cdp.evaluate(`document.querySelector('#gate-rule\\\\.tests-only-when-asked .badge').textContent === 'shadow'`), "shadow rules are marked on the board");
  check(await cdp.evaluate(`document.querySelectorAll('#flags .row.flag').length === 2`), "unlabelled flags wait for a label");
  await cdp.evaluate(`document.querySelector('#flags [data-id="verification.gate_77d"] .btn.quiet').click(); true`); await delay(150);
  await fill("#flags .editor.open form", { rationale: "The colour is in an SVG asset." });
  await until(`document.querySelectorAll('#flags .row.flag').length === 1`, "the flag was not labelled");
  await cdp.evaluate(`document.querySelector('#run-btn').click(); true`); await delay(150);
  check(await cdp.evaluate(`document.querySelector('#run-btn').classList.contains('busy')`), "run shows a busy state");
  await until(`document.querySelector('#checks .board.landed')`, "the run did not land");
  check(await cdp.evaluate(`[...document.querySelectorAll('#checks .brow .state')].every(s => !/not run/.test(s.textContent) || s.closest('.brow').id.includes('draft'))`), "results are current after a run");

  // Requests: answer the raised hand.
  await open("requests"); await delay(150);
  check(await cdp.evaluate(`!document.querySelector('#requests .req').classList.contains('done')`), "open requests come first");
  await fill("#requests .req form", { content: "Yes, parse to integer cents at the webhook." });
  await until(`document.querySelectorAll('#requests .req.done').length === 2`, "the answer was not recorded");

  // Changelog: newest first, accept a draft, search, as-of.
  await open("changelog"); await delay(150);
  check(await cdp.evaluate(`/Answered ledger-7kq/.test(document.querySelector('.entry .t').textContent)`), "the changelog is newest first");
  await cdp.evaluate(`[...document.querySelectorAll('.entry .act .btn')].find(b => b.textContent === 'Accept').click(); true`);
  await until(`document.querySelector('#confirm-review')`, "accept did not show its review");
  await cdp.evaluate(`document.querySelector('#confirm-review').click(); true`);
  await until(`document.querySelector('#draft-count').textContent === '1'`, "the draft was not accepted");
  await cdp.evaluate(`(() => { const i=document.querySelector('#cl-q'); i.value='exemplar'; i.dispatchEvent(new Event('input')); return true; })()`); await delay(500);
  check(await cdp.evaluate(`document.querySelectorAll('.entry').length === 1`), "search filters entries");
  await cdp.evaluate(`(() => { const i=document.querySelector('#cl-q'); i.value=''; i.dispatchEvent(new Event('input')); return true; })()`); await delay(500);
  await cdp.evaluate(`(() => { const a=document.querySelector('#cl-asof'); a.value='2026-09-23T12:00'; a.dispatchEvent(new Event('change')); return true; })()`); await delay(300);
  check(await cdp.evaluate(`document.querySelectorAll('.entry').length === 3 && /as of/.test(document.querySelector('.asof-note').textContent)`), "as-of shows the trail as it stood");
  check(await cdp.evaluate(`document.querySelector('.entry details pre') !== null`), "exact records sit behind disclosure");

  // Keyboard: the rail is a tablist with arrow navigation.
  await cdp.evaluate(`document.querySelector('#tab-home').focus(); document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true})); true`); await delay(100);
  check(await cdp.evaluate(`document.activeElement.id === 'tab-rules'`), "arrow keys move between views");

  // Widths: no horizontal overflow at any agreed width, in both themes.
  for (const theme of ["dark", "light"]) {
    await cdp.evaluate(`document.querySelector('.theme button[data-theme=${theme}]').click(); true`);
    for (const width of [320, 390, 768, 1280]) {
      await cdp.send("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: width < 768 });
      for (const tab of ["home", "rules", "checks", "changelog", "requests"]) {
        await open(tab); await delay(80);
        const overflow = await cdp.evaluate(`document.documentElement.scrollWidth - window.innerWidth`);
        check(overflow <= 1, `${tab} fits at ${width}px in ${theme} (overflow ${overflow}px)`);
        if (width === 390 || width === 1280) await shot(`${tab}-${width}-${theme}`, width);
      }
    }
  }
  check(cdp.errors.length === 0, `no page errors: ${cdp.errors.join("\n")}`);
  console.log(`ok ${assertions} assertions · screenshots in ${output}`);
} finally {
  browser.kill();
}

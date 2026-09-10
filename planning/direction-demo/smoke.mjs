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
  const demo = (state) => cdp.evaluate(`document.querySelector('input[name=demo][value=${state}]').click(); true`);
  const open = (tab) => cdp.evaluate(`document.querySelector("#tab-${tab}").click(); true`);

  // Fresh project: onboarding only, later views gated.
  await demo("fresh"); await delay(150);
  check(await cdp.evaluate(`document.querySelectorAll('#dashboard .panel').length === 1`), "fresh dashboard holds one onboarding panel");
  check(await cdp.evaluate(`!document.querySelector('#dashboard #h-metrics') && !document.querySelector('#dashboard #h-gates')`), "fresh dashboard shows no metric or gate panels");
  check(await cdp.evaluate(`document.querySelector('#tab-checks').getAttribute('aria-disabled') === 'true' && document.querySelector('#tab-changelog').getAttribute('aria-disabled') === 'true'`), "checks and changelog are gated before init");
  check(await cdp.evaluate(`document.querySelector('#agreement-state').textContent === 'not initialised'`), "top bar says not initialised");
  await open("checks"); check(await cdp.evaluate(`document.querySelector('#dashboard').hidden === false`), "gated view is not opened");
  await open("foundations"); check(await cdp.evaluate(`!!document.querySelector('#foundations #init-form')`), "fresh foundations show the first-agreement form");

  // Established project: five stages, honest states, one primary action.
  await demo("established"); await delay(150);
  check(await cdp.evaluate(`document.querySelectorAll('#foundations .stage').length === 5`), "foundations render exactly five stages");
  check(await cdp.evaluate(`[...document.querySelectorAll('#foundations .stage .ph h2')].map(h=>h.firstChild.textContent.trim()).join('|') === 'Mission|Core values|Key metrics|Rules & guidelines|Gates'`), "stages are in the agreed order");
  check(await cdp.evaluate(`document.querySelectorAll('#dashboard .attn .btn.primary').length === 1`), "dashboard has exactly one primary action");
  check(await cdp.evaluate(`[...document.querySelectorAll('#dashboard .state.pass')].every(s => !/unknown|not/.test(s.textContent))`), "unknown states are never green");
  check(await cdp.evaluate(`!!document.querySelector('#dashboard .state.warn')`), "not-observed and stale states are shown as amber");

  // Edit → review → draft keeps the accepted record in force.
  await open("foundations");
  await cdp.evaluate(`document.querySelector('[data-edit="standard.browser"]').click(); true`); await delay(250);
  check(await cdp.evaluate(`document.querySelector('.editor.open form') !== null`), "edit opens inline, not in a modal");
  await cdp.evaluate(`(() => { const f=document.querySelector('form[data-form]'); f.querySelector('[name=content]').value='Browser acceptance journey incl. retry'; f.querySelector('[name=rationale]').value='Retry regressions escaped review'; f.querySelector('[name=effect]').value='Retry path is gated'; f.requestSubmit(); return true; })()`); await delay(250);
  check(await cdp.evaluate(`document.querySelector('.review .diff .after') !== null && /shared/.test(document.querySelector('.review .effects').textContent)`), "review shows before/after and effects");
  await shot("review", 1440);
  await cdp.evaluate(`document.querySelector('[data-confirm]').click(); true`); await delay(250);
  check(await cdp.evaluate(`document.querySelector('#draft-count').textContent === '2'`), "draft count increments");
  check(await cdp.evaluate(`/draft v4 pending/.test(document.querySelector('[data-rec="standard.browser"] .ver').textContent)`), "accepted record shows the pending draft");
  check(await cdp.evaluate(`document.querySelector('[data-rec="standard.browser"] .c').textContent === 'Browser acceptance journey'`), "accepted content stays in force");
  await open("checks");
  check(await cdp.evaluate(`document.querySelector('#gate-standard\\\\.browser .state').classList.contains('fail')`), "checks still show the accepted gate's live failure");
  check(await cdp.evaluate(`document.querySelector('#det-standard\\\\.browser pre.brief') !== null`), "failing gate carries a repair brief");

  // Run checks updates rows in place.
  await cdp.evaluate(`document.querySelector('#run-form').requestSubmit(); true`); await delay(150);
  check(await cdp.evaluate(`document.querySelector('#run-btn').classList.contains('busy')`), "run shows a busy state");
  await delay(1100);
  check(await cdp.evaluate(`!document.querySelector('.state.warn') || [...document.querySelectorAll('#checks .brow .state')].every(s => !/stale/.test(s.textContent))`), "stale results become current after a run");
  check(await cdp.evaluate(`document.querySelector('#gate-standard\\\\.regex .state').classList.contains('draft')`), "draft gates are not run");

  // Changelog: newest first, search, as-of.
  await open("changelog");
  check(await cdp.evaluate(`document.querySelector('.entry .t').textContent.startsWith('Gate revised')`), "changelog is newest first and includes the new draft");
  await cdp.evaluate(`(() => { const i=document.querySelector('#cl-q'); i.value='metric'; i.dispatchEvent(new Event('input')); return true; })()`); await delay(100);
  check(await cdp.evaluate(`document.querySelectorAll('.entry').length === 1`), "search filters entries");
  await cdp.evaluate(`(() => { const i=document.querySelector('#cl-q'); i.value=''; i.dispatchEvent(new Event('input')); return true; })()`); await delay(100);
  await cdp.evaluate(`(() => { const a=document.querySelector('#cl-asof'); a.value='2026-09-03T12:00'; a.dispatchEvent(new Event('change')); return true; })()`); await delay(100);
  check(await cdp.evaluate(`document.querySelectorAll('.entry.future').length === 4 && document.querySelectorAll('.entry:not(.future)').length === 3`), "as-of dims later entries and keeps earlier ones");
  check(await cdp.evaluate(`document.querySelector('.entry details pre') !== null`), "exact records sit behind disclosure");

  // Keyboard: rail is a tablist with arrow navigation.
  await cdp.evaluate(`document.querySelector('#tab-dashboard').focus(); document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true})); true`);
  check(await cdp.evaluate(`document.activeElement.id === 'tab-foundations'`), "arrow keys move between views");

  // Widths: no horizontal overflow at any agreed width.
  for (const width of [320, 390, 768, 1024, 1440]) {
    await cdp.send("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: width < 768 });
    for (const tab of ["dashboard", "foundations", "checks", "changelog"]) {
      await open(tab); await delay(80);
      const overflow = await cdp.evaluate(`document.documentElement.scrollWidth - window.innerWidth`);
      check(overflow <= 1, `${tab} fits at ${width}px (overflow ${overflow}px)`);
      if (width === 390 || width === 1440) await shot(`${tab}-${width}`, width);
    }
  }
  check(cdp.errors.length === 0, `no page errors: ${cdp.errors.join("\n")}`);
  console.log(`ok ${assertions} assertions · screenshots in ${output}`);
} finally {
  browser.kill();
}

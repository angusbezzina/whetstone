#!/usr/bin/env node

import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";

const dashboardUrl = process.env.WH_DASHBOARD_URL;
assert.ok(dashboardUrl, "WH_DASHBOARD_URL is required");
const projectRoot = process.env.WH_PROJECT_ROOT;
assert.ok(projectRoot, "WH_PROJECT_ROOT is required");
const chrome = [
  process.env.CHROME_BIN,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].filter(Boolean).find(existsSync);
if (!chrome) {
  console.log("SKIP live dashboard browser test: Chrome/Chromium is unavailable");
  process.exit(77);
}

const profile = mkdtempSync(join(tmpdir(), "whetstone-dashboard-live-"));
const browser = spawn(chrome, [
  "--headless=new",
  "--remote-debugging-port=0",
  `--user-data-dir=${profile}`,
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-background-networking",
  "--disable-dev-shm-usage",
  "--disable-gpu",
  "--no-sandbox",
  "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });

const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

async function waitForFile(path, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (existsSync(path)) return readFileSync(path, "utf8");
    if (browser.exitCode !== null) throw new Error(`Chrome exited early with ${browser.exitCode}`);
    await delay(25);
  }
  throw new Error(`Timed out waiting for ${path}`);
}

class Cdp {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.sequence = 0;
    this.pending = new Map();
    this.events = new Map();
  }

  async open() {
    await new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", reject, { once: true });
    });
    this.socket.addEventListener("message", ({ data }) => {
      const message = JSON.parse(data);
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        if (message.error) pending.reject(new Error(JSON.stringify(message.error)));
        else pending.resolve(message.result);
        return;
      }
      const listeners = this.events.get(message.method) ?? [];
      this.events.delete(message.method);
      for (const listener of listeners) listener(message.params);
    });
  }

  send(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  event(method) {
    return new Promise((resolve) => {
      const listeners = this.events.get(method) ?? [];
      listeners.push(resolve);
      this.events.set(method, listeners);
    });
  }

  async evaluate(expression) {
    const result = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
    return result.result.value;
  }
}

let cdp;
try {
  const activePort = await waitForFile(join(profile, "DevToolsActivePort"));
  const port = activePort.split("\n")[0];
  let page;
  for (let attempt = 0; attempt < 400; attempt += 1) {
    try {
      const pages = await fetch(`http://127.0.0.1:${port}/json/list`).then((response) => response.json());
      page = pages.find((candidate) => candidate.type === "page" && !candidate.url.startsWith("chrome-extension:"));
      if (page) break;
    } catch {}
    await delay(25);
  }
  assert.ok(page, "Chrome did not expose a page target");
  cdp = new Cdp(page.webSocketDebuggerUrl);
  await cdp.open();
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Network.enable");

  async function waitFor(expression, message, timeout = 10_000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      if (await cdp.evaluate(expression)) return;
      await delay(25);
    }
    const debug = await cdp.evaluate(`({
      href: location.href,
      state: document.querySelector("#state")?.textContent,
      command: document.querySelector("#command-state")?.textContent,
      announcement: document.querySelector("#announcement")?.textContent,
      change: { hidden: document.querySelector("#change-form")?.hidden, kind: document.querySelector("#change-form")?.elements.kind?.value },
      metricButton: document.querySelector("#foundation-metrics")?.closest("article")?.querySelector(".node-edit")?.outerHTML,
      body: document.body?.textContent?.slice(0, 500),
    })`);
    throw new Error(`${message}: ${JSON.stringify(debug)}`);
  }

  const loaded = cdp.event("Page.loadEventFired");
  await cdp.send("Page.navigate", { url: dashboardUrl });
  await loaded;
  await waitFor('document.querySelector("#dashboard-mission")?.textContent.length > 0', "dashboard did not render");
  assert.equal(await cdp.evaluate('document.querySelector("#state").hidden'), true);
  assert.equal(await cdp.evaluate('document.querySelector("#tab-dashboard").getAttribute("aria-selected")'), "true");
  assert.equal(await cdp.evaluate('document.querySelectorAll("[role=tab]").length'), 4);

  await cdp.evaluate('document.querySelector("#tab-foundations").click(); document.querySelector("#edit").click()');
  await waitFor('!document.querySelector("#init-form").hidden && document.querySelector("#command-record").textContent.includes("expected_revision")', "edit mode did not bind onboarding base");
  await cdp.evaluate(`(() => {
    const form = document.querySelector("#init-form");
    const values = {
      mission: "Keep human and agent work aligned",
      desired_outcome: "Surface only accountable decisions while safeguards repair routine drift",
      values: "Evidence before assertion; private before shared",
      philosophy: "Typed deterministic services own replayable work; skills own judgment",
      owner: "Browser smoke owner",
      initial_safeguard: "Never weaken a failing gate to claim success",
      safeguard_scope: "All repository changes",
      revision_triggers: "Mission, architecture, or repeated repair-loop friction",
    };
    for (const [name, value] of Object.entries(values)) form.elements[name].value = value;
    form.elements.mission.dispatchEvent(new Event("input", { bubbles: true }));
    form.querySelector('button[type="submit"]').click();
  })()`);
  await waitFor('document.querySelector("#review-dialog").open', "onboarding exact review did not open");
  assert.equal(await cdp.evaluate('document.querySelector("#review-record").textContent.includes("effects_on_confirm")'), true);
  assert.equal(await cdp.evaluate('document.querySelector("#review-record").textContent.includes("team_share")'), true);
  await cdp.evaluate('document.querySelector("#review-confirm").click()');
  await waitFor('document.querySelector("#mission").textContent === "Keep human and agent work aligned"', "persisted mission was not projected");
  assert.equal(await cdp.evaluate('document.querySelector("#foundation-values").textContent'), "Evidence before assertion; private before shared");
  assert.equal(await cdp.evaluate('document.querySelector("#foundation-philosophy").textContent.includes("Typed deterministic services")'), true);
  assert.equal(await cdp.evaluate('document.querySelector("#foundation-safeguard").textContent'), "Never weaken a failing gate to claim success");
  assert.equal(await cdp.evaluate('document.querySelector("#command-state").textContent'), "command needs_input");
  assert.equal(await cdp.evaluate('document.querySelector("#metric-count").textContent'), "Not configured");
  assert.equal(await cdp.evaluate('document.querySelector("#gate-count").textContent'), "Not configured");
  assert.equal(await cdp.evaluate('document.querySelector("#attention-kind").textContent'), "human decision");
  assert.equal(await cdp.evaluate('document.querySelector("#attention-title").textContent'), "No key metric is defined");
  assert.equal(await cdp.evaluate('document.querySelector("#attention-meta").textContent'), "For: project owner");
  assert.equal(await cdp.evaluate('document.querySelector("#attention-continue").textContent'), "Open foundations");
  assert.equal(await cdp.evaluate('document.querySelectorAll("#foundation-view .foundation-node").length >= 7'), true);

  await cdp.evaluate('document.querySelector("#foundation-metrics").closest("article").querySelector(".node-edit").click()');
  await waitFor('!document.querySelector("#change-form").hidden && document.querySelector("#change-form").elements.kind.value === "metric"', "metric editor did not open");
  await cdp.evaluate(`(() => {
    const form = document.querySelector("#change-form");
    const values = { content: "Aligned work completed", def_source_system: "project-review", def_source_locator: "weekly-scorecard", def_cohort: "current project", def_window: "30 days", def_direction: "increase", def_threshold: "90%", def_freshness_seconds: "86400", rationale: "Measure the mission outcome", source: "owner:browser-smoke", expected_effect: "Expose outcome drift", impact: "Local metric draft" };
    for (const [name, value] of Object.entries(values)) form.elements.namedItem(name).value = value;
    form.elements.content.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector("#change-submit").click();
  })()`);
  await waitFor('document.querySelector("#review-dialog").open', "metric review did not open");
  assert.equal(await cdp.evaluate('document.querySelector("#review-record").textContent.includes("metric_definition")'), true);
  await cdp.evaluate('document.querySelector("#review-confirm").click()');
  await waitFor('document.querySelector("#metric-count").textContent === "1 defined"', "metric definition was not projected");
  assert.equal(await cdp.evaluate('document.querySelector("#foundation-metrics").closest("li").textContent.includes("Aligned work completed")'), true);

  await cdp.evaluate('document.querySelector("#foundation-gates").closest("article").querySelector(".node-edit").click()');
  await waitFor('!document.querySelector("#change-form").hidden && document.querySelector("#change-form").elements.kind.value === "standard"', "gate editor did not open");
  await cdp.evaluate(`(() => {
    const form = document.querySelector("#change-form");
    const values = { content: "Rust tests must pass", def_strength: "must", def_enforcement: "test", def_value: "cargo test", rationale: "Prevent regressions", source: "owner:browser-smoke", expected_effect: "Block known failures", impact: "Local gate draft" };
    for (const [name, value] of Object.entries(values)) form.elements.namedItem(name).value = value;
    form.elements.content.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector("#change-submit").click();
  })()`);
  await waitFor('document.querySelector("#review-dialog").open', "gate review did not open");
  assert.equal(await cdp.evaluate('document.querySelector("#review-record").textContent.includes("standard")'), true);
  await cdp.evaluate('document.querySelector("#review-confirm").click()');
  await waitFor('document.querySelector("#gate-count").textContent === "1 configured"', "gate definition was not projected");
  assert.equal(await cdp.evaluate('document.querySelector("#foundation-gates").closest("li").textContent.includes("Rust tests must pass")'), true);
  await cdp.evaluate('document.querySelector("#tab-enforcement").click()');
  assert.equal(await cdp.evaluate('document.querySelector("#current-context").textContent.includes("Test: cargo test")'), true);
  assert.equal(await cdp.evaluate('document.querySelector("#current-context").textContent.includes("not checked")'), true);
  assert.equal(await cdp.evaluate('document.querySelector("#agent-handoff").hidden'), false);
  assert.equal(await cdp.evaluate(`document.querySelector("#agent-instruction").textContent.includes(${JSON.stringify(projectRoot)})`), true);
  assert.equal(await cdp.evaluate('document.querySelector("#agent-instruction").textContent.includes("Never weaken a failing gate")'), true);
  assert.equal(await cdp.evaluate('document.querySelector("#agent-instruction").textContent.includes("same worker")'), true);

  const reloaded = cdp.event("Page.loadEventFired");
  await cdp.send("Page.reload");
  await reloaded;
  await waitFor('document.querySelector("#dashboard-mission")?.textContent === "Keep human and agent work aligned"', "dashboard mission did not survive reload");
  await cdp.evaluate('document.querySelector("#tab-foundations").click()');
  await waitFor('document.querySelector("#mission")?.textContent === "Keep human and agent work aligned"', "foundation mission did not survive reload");
  assert.equal(await cdp.evaluate('document.querySelector("#init-form").elements.mission.value'), "Keep human and agent work aligned");

  await cdp.evaluate('document.querySelector("#tab-foundations").click(); document.querySelector("#edit").click()');
  await waitFor('!document.querySelector("#change-form").hidden', "change form did not open");
  assert.equal(await cdp.evaluate('document.querySelector("#change-base").textContent'), "v1");
  await cdp.evaluate(`(() => {
    const form = document.querySelector("#change-form");
    form.elements.content.value = "Keep human and agent work visibly aligned";
    form.elements.rationale.value = "Keep CLI and UI semantics identical";
    form.elements.source.value = "owner:browser-smoke";
    form.elements.expected_effect.value = "One replayable command path";
    form.elements.impact.value = "Local mission draft only";
    form.elements.content.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector("#change-submit").click();
  })()`);
  assert.equal(await cdp.evaluate('document.querySelector("#change-form").dataset.dirty'), "true");
  await waitFor('document.querySelector("#review-dialog").open', "typed change preview did not open");
  assert.equal(await cdp.evaluate('document.querySelector("#review-record").textContent.includes("preview_only")'), true);
  assert.equal(await cdp.evaluate('document.querySelector("#review-record").textContent.includes("before")'), true);
  assert.equal(await cdp.evaluate('document.querySelector("#review-record").textContent.includes("private_record_write_on_confirm")'), true);
  await cdp.evaluate('document.querySelector("#review-confirm").click()');
  await waitFor('document.querySelector("#command-state").textContent === "command success"', "change was not recorded");
  await waitFor('document.querySelector("#mission").textContent === "Keep human and agent work visibly aligned"', "established foundation was not revised");
  assert.equal(await cdp.evaluate('document.querySelector("#foundation-view .foundation-node").dataset.state'), "local_draft");
  await cdp.evaluate('document.querySelector("#tab-decisions").click()');
  await waitFor('document.querySelector("#decision-history").textContent.includes("Change mission.project")', "change did not reach the decision log");

  await cdp.evaluate('document.querySelector("#tab-foundations").click(); document.querySelector("#foundation-view .node-edit").click()');
  await waitFor('!document.querySelector("#change-form").hidden && document.querySelector("#change-base").textContent === "v2"', "stale-test base was not loaded");
  const concurrent = await cdp.evaluate(`(async () => {
    const csrf = sessionStorage.getItem("whetstone_csrf");
    const send = async (body) => (await fetch("/api/command", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Whetstone-CSRF": csrf },
      body: JSON.stringify(body),
    })).json();
    const base = await send({ workflow: "change", request_id: "browser-concurrent", record_id: "mission.project" });
    const exact = {
      workflow: "change", request_id: "browser-concurrent", kind: "mission", record_id: "mission.project",
      content: "Concurrent mission decision", rationale: "Exercise optimistic concurrency", source: "test:browser",
      expected_effect: "Advance the revision", impact: "Local fixture", examples: [], conflicts: [],
      expected_revision: base.expected_revision, resume_token: base.resume_token, preview: false,
    };
    const accepted = await send(exact);
    const conflict = await send({ ...exact, content: "Conflicting reuse" });
    return { accepted, conflict };
  })()`);
  assert.equal(concurrent.accepted.state, "success");
  assert.equal(concurrent.conflict.state, "conflict");
  await cdp.evaluate(`(() => {
    const form = document.querySelector("#change-form");
    form.elements.content.value = "Stale browser mission";
    form.elements.rationale.value = "Must be rejected";
    form.elements.source.value = "test:browser";
    form.elements.expected_effect.value = "No write";
    form.elements.impact.value = "None";
    document.querySelector("#change-submit").click();
  })()`);
  await waitFor('document.querySelector("#command-state").textContent === "command stale"', "stale preview was not surfaced");
  assert.equal(await cdp.evaluate('document.querySelector("#review-dialog").open'), false);

  await cdp.evaluate('document.querySelector("#tab-enforcement").click(); document.querySelector("#enable-checks").click()');
  await waitFor('!document.querySelector("#check-form").hidden', "check form did not open");
  await cdp.evaluate('document.querySelector("#check-form button[type=submit]").click()');
  await waitFor('document.querySelector("#command-state").textContent === "command unknown"', "unknown check outcome was not preserved");
  assert.equal(await cdp.evaluate('document.querySelector("#command-record").textContent.includes("unknown")'), true);

  for (const width of [320, 390, 768, 1024]) {
    await cdp.send("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: width < 768 });
    for (const tab of ["dashboard", "foundations", "enforcement", "decisions"]) {
      const overflow = await cdp.evaluate(`(() => {
        document.querySelector(${JSON.stringify(`#tab-${tab}`)}).click();
        return {
          tab: ${JSON.stringify(tab)},
          fits: document.documentElement.scrollWidth - window.innerWidth <= 5,
          width: window.innerWidth,
          documentWidth: document.documentElement.scrollWidth,
        };
      })()`);
      assert.equal(overflow.fits, true, `live dashboard overflows: ${JSON.stringify(overflow)}`);
    }
  }
  await cdp.send("Network.clearBrowserCookies");
  const denied = await cdp.evaluate(`(async () => {
    const response = await fetch("/api/command", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Whetstone-CSRF": sessionStorage.getItem("whetstone_csrf") },
      body: JSON.stringify({ workflow: "change", request_id: "browser-denied", record_id: "guidance.denied" }),
    });
    return { status: response.status, body: await response.json() };
  })()`);
  assert.equal(denied.status, 401);
  assert.equal(denied.body.reason_code, "authentication_required");

  console.log("PASS live dashboard action/onboarding/change/check/stale/conflict/permission/persistence");
} finally {
  cdp?.socket.close();
  browser.kill("SIGTERM");
  await new Promise((resolve) => {
    if (browser.exitCode !== null) resolve();
    else browser.once("exit", resolve);
  });
  rmSync(profile, { recursive: true, force: true });
}

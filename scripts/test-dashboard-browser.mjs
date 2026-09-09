#!/usr/bin/env node

import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const chromeCandidates = [
  process.env.CHROME_BIN,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].filter(Boolean);
const chrome = chromeCandidates.find(existsSync);
if (!chrome) {
  console.log("SKIP dashboard browser test: Chrome/Chromium is unavailable");
  process.exit(77);
}

const assets = {
  "/": ["text/html; charset=utf-8", readFileSync(join(root, "assets/dashboard/index.html"))],
  "/app.css": ["text/css; charset=utf-8", readFileSync(join(root, "assets/dashboard/app.css"))],
  "/app.js": ["text/javascript; charset=utf-8", readFileSync(join(root, "assets/dashboard/app.js"))],
};

const maliciousMission = '<img src=x onerror="globalThis.whetstoneXss=true">Own the outer loop';
const refreshedMission = "Fresh partial mission from exact base";
const counters = { cancel: 0, agree: 0, inspectQueries: 0, inspections: 0, lastInspect: {} };

function agreementRecord(recordType, record, id) {
  return {
    reference: { id, revision: 1, digest: `sha256:${"a".repeat(64)}` },
    recorded_at: "2026-09-09T12:00:00Z",
    redaction_reason: null,
    links: [],
    record: {
      schema_version: 1,
      id,
      revision: 1,
      scope: { project: "project-browser" },
      owner: { kind: "local_user", stable_id: "local:browser", display_name: "Owner" },
      provenance: {
        kind: "human_authored",
        recorded_by: { kind: "local_user", stable_id: "local:browser", display_name: "Owner" },
        recorded_at: "2026-09-09T12:00:00Z",
        sources: [{ system: "whetstone_cli", locator: "owner_input" }],
        authority: "owner_authored",
      },
      idempotency_key: `browser:${id}`,
      record_type: recordType,
      record,
    },
  };
}

function inspection(mode = "normal") {
  const missionStatement = counters.inspections > 1 ? refreshedMission : maliciousMission;
  const mission = agreementRecord(
    "mission",
    { statement: missionStatement, desired_outcomes: ["Keep work aligned"] },
    "mission.project",
  );
  const standard = agreementRecord(
    "standard",
    {
      statement: "Do not weaken a failing gate.",
      rationale: "A green result must retain its meaning.",
      strength: "must",
      enforcement: { enforcement: "test", command_ref: "cargo test" },
      examples: [],
    },
    "standard.gates",
  );
  const value = agreementRecord("core_value", { name: "Evidence", description: "Evidence before assertion" }, "value.evidence");
  const philosophy = agreementRecord("implementation_philosophy", { statement: "Typed services own replayable work", review_triggers: ["Architecture changes"] }, "philosophy.project");
  const items = mode === "empty" ? [] : [mission, standard];
  const nextAction = mode === "draft" ? {
    title: "Inspect your 2 local draft proposal(s)",
    explanation: "These drafts remain private and inactive until reviewed.",
    actor: "project owner",
    permitted_next_action: "Open Decisions to inspect the drafts.",
    route: "decisions",
    agent_instruction: null,
  } : {
    title: "Test your first safeguard",
    explanation: "Your project agreement is saved. An agent still needs to prove the repair loop.",
    actor: "your coding agent",
    permitted_next_action: "Give the repair-proof handoff to your agent.",
    route: "enforcement",
    agent_instruction: "In /fixture/project, run the exact repair proof.",
  };
  return {
    schema: "whetstone.command-response.v1",
    schema_version: 1,
    request_id: "browser-inspect",
    workflow: "dash",
    state: mode === "unknown" ? "unknown" : "success",
    summary: mode === "unknown" ? "History is unknown." : "Project shape is ready.",
    evidence: [],
    blocking_questions: [],
    permitted_actions: ["wh change", "wh check"],
    data: {
      setup: { facts: [] },
      progress: {
        discovery: "complete",
        agreement: "approved",
        private_store: "ready",
        feedback_loop: "needs_proof",
        setup_complete: false,
        agreement_revision: 7,
        proof_status: "Repair proof has not been established.",
      },
      progress_state: "available",
      progress_detail: null,
      history: {
        snapshot_digest: `sha256:${"b".repeat(64)}`,
        active_context: { items, next: null },
        decision_history: { items, next: null },
      },
      history_state: mode === "unknown" ? "unavailable" : "available",
      history_detail: mode === "unknown" ? "The source could not be read." : null,
      current: mode === "missing-current" ? null : {
        local_agreement: {
          state: "owner_approved_private",
          team_activation: "not_configured",
          mission: mission.record,
          core_values: value.record,
          implementation_philosophy: philosophy.record,
          initial_safeguard: standard.record,
        },
        workspace: {
          latest_verification: null,
          verification_currentness: "unknown_without_exact_recheck",
          latest_repair: null,
          repair_currentness: "unproven",
          latest_observation: null,
          required_policy: "not_configured",
          installed_state: "private_local_store",
          experimental_local_drafts: mode === "draft" ? 2 : 0,
          delivered_context_revision: null,
          needed_decision: null,
          next_action: nextAction,
          pending_operations: ["prove one current repair"],
        },
      },
      workflows: [
        ["init", "available", "inspect or establish private agreement"],
        ["dash", "available", "inspect local system and history"],
        ["change", "available", "propose a bounded local change"],
        ["check", "available", "verify and return repair feedback"],
        ["pull", "unavailable", "no remote changes are received"],
        ["push", "unavailable", "nothing is published"],
      ].map(([name, state, effect]) => ({ name, state, effect })),
      lean_baseline_revision: "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48",
      read_only: true,
    },
  };
}

function json(response, status = 200, headers = {}) {
  return [status, { "content-type": "application/json", ...headers }, JSON.stringify(response)];
}

const server = createServer((request, response) => {
  const chunks = [];
  request.on("data", (chunk) => chunks.push(chunk));
  request.on("end", () => {
    let outgoing;
    if (request.method === "GET" && assets[request.url]) {
      const [contentType, body] = assets[request.url];
      outgoing = [200, { "content-type": contentType }, body];
    } else if (request.method === "POST" && request.url === "/session/bootstrap") {
      outgoing = json(
        { csrf_token: "browser-csrf", mode: "inspect" },
        200,
        { "set-cookie": "whetstone_session=browser; HttpOnly; SameSite=Strict; Path=/" },
      );
    } else if (request.method === "POST" && request.url === "/session/edit") {
      outgoing = json({ mode: "edit" });
    } else if (request.url === "/api/inspect") {
      counters.inspections += 1;
      let body = {};
      if (chunks.length) body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      counters.lastInspect = body;
      if (Object.keys(body).length) counters.inspectQueries += 1;
      if (body.search === "__error__") {
        outgoing = json({ state: "unknown", summary: "Synthetic inspection failure." }, 500);
      } else if (body.search === "__empty__") {
        outgoing = json(inspection("empty"));
      } else if (body.search === "__unknown__") {
        outgoing = json(inspection("unknown"));
      } else if (body.search === "__missing_current__") {
        outgoing = json(inspection("missing-current"));
      } else if (body.search === "__draft__") {
        outgoing = json(inspection("draft"));
      } else {
        outgoing = json(inspection());
      }
    } else if (request.method === "POST" && request.url === "/api/command") {
      const body = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      if (body.workflow === "init" && body.action === "cancel") {
        counters.cancel += 1;
        outgoing = json({ state: "success", summary: "Cancelled.", data: {} });
      } else if (body.workflow === "init" && body.action === "inspect") {
        outgoing = json({
          state: "needs_input",
          summary: "Review owner decisions.",
          expected_revision: 7,
          resume_token: "exact-browser-base",
          data: {},
        });
      } else if (body.workflow === "init" && body.action === "agree") {
        counters.agree += 1;
        outgoing = json({ state: "stale", summary: "The exact base changed.", data: {} });
      } else if (body.workflow === "check") {
        outgoing = json({ state: "unknown", summary: "Required evidence is unavailable.", data: {} });
      } else {
        outgoing = json({ state: "needs_input", summary: "More input is required.", data: {} });
      }
    } else {
      outgoing = [404, { "content-type": "text/plain" }, "not found"];
    }
    response.writeHead(outgoing[0], { "cache-control": "no-store", ...outgoing[1] });
    response.end(outgoing[2]);
  });
});

await new Promise((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
const address = server.address();
const origin = `http://127.0.0.1:${address.port}`;
const profile = mkdtempSync(join(tmpdir(), "whetstone-dashboard-browser-"));
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

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}

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
    await new Promise((resolveOpen, rejectOpen) => {
      this.socket.addEventListener("open", resolveOpen, { once: true });
      this.socket.addEventListener("error", rejectOpen, { once: true });
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
      for (const resolveEvent of listeners) resolveEvent(message.params);
    });
  }

  send(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolveSend, rejectSend) => {
      this.pending.set(id, { resolve: resolveSend, reject: rejectSend });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  event(method) {
    return new Promise((resolveEvent) => {
      const listeners = this.events.get(method) ?? [];
      listeners.push(resolveEvent);
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
  let pages;
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      pages = await fetch(`http://127.0.0.1:${port}/json/list`).then((response) => response.json());
      if (pages.length) break;
    } catch {}
    await delay(25);
  }
  const page = pages?.find((candidate) => candidate.type === "page" && !candidate.url.startsWith("chrome-extension:"));
  assert.ok(page, "Chrome did not expose a page target");
  cdp = new Cdp(page.webSocketDebuggerUrl);
  await cdp.open();
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  const loaded = cdp.event("Page.loadEventFired");
  await cdp.send("Page.navigate", { url: `${origin}/#bootstrap=browser-bootstrap` });
  await loaded;

  async function waitFor(expression, message, timeout = 5_000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      if (await cdp.evaluate(expression)) return;
      await delay(25);
    }
    const debug = await cdp.evaluate(`({
      href: location.href,
      readyState: document.readyState,
      title: document.title,
      state: document.querySelector("#state")?.textContent,
      body: document.body?.textContent?.slice(0, 300),
    })`);
    throw new Error(`${message}: ${JSON.stringify(debug)}`);
  }

  await waitFor(
    `location.origin === ${JSON.stringify(origin)} && document.querySelector("#dashboard-mission")?.textContent === ${JSON.stringify(maliciousMission)}`,
    "dashboard did not render",
  );
  assert.equal(await cdp.evaluate('document.querySelector("#state").hidden'), true);
  assert.equal(await cdp.evaluate('document.querySelectorAll("[role=tab]").length'), 4);
  assert.equal(await cdp.evaluate('document.querySelector("#mission").textContent'), maliciousMission);
  assert.equal(await cdp.evaluate('document.querySelector("#mission img") === null'), true);
  assert.equal(await cdp.evaluate("globalThis.whetstoneXss === undefined"), true);

  assert.equal(await cdp.evaluate('document.querySelector("#attention-title").textContent'), "Test your first safeguard");
  await cdp.evaluate('document.querySelector("#attention-continue").click()');
  assert.equal(await cdp.evaluate('document.querySelector("#tab-enforcement").getAttribute("aria-selected")'), "true");
  assert.equal(await cdp.evaluate('document.activeElement.id'), "agent-handoff");
  assert.equal(await cdp.evaluate('document.querySelector("#agent-instruction").textContent'), "In /fixture/project, run the exact repair proof.");
  await cdp.evaluate('document.querySelector("#tab-dashboard").focus()');
  await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", key: "ArrowRight", code: "ArrowRight" });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "ArrowRight", code: "ArrowRight" });
  assert.equal(await cdp.evaluate('document.activeElement.id'), "tab-foundations");
  assert.equal(await cdp.evaluate('document.querySelector("#tab-foundations").getAttribute("aria-selected")'), "true");

  await cdp.evaluate('document.querySelector("#tab-foundations").click()');
  await cdp.evaluate('document.querySelector("#edit").click()');
  await waitFor('!document.querySelector("#init-form").hidden', "edit mode did not expose forms");
  assert.equal(
    await cdp.evaluate('document.querySelector("#init-form").elements.mission.value'),
    refreshedMission,
    "edit mode must refresh partial values from the exact bound base",
  );
  await cdp.evaluate('document.querySelector("#cancel-init").click()');
  await waitFor('document.querySelector("#announcement").textContent.includes("cancelled")', "cancel was not announced");
  assert.equal(counters.cancel, 1);
  assert.equal(counters.agree, 0);

  await cdp.evaluate(`(() => {
    const form = document.querySelector("#init-form");
    form.elements.mission.value = "A precise mission";
    form.elements.desired_outcome.value = "A visible outcome";
    form.elements.values.value = "Evidence first";
    form.elements.philosophy.value = "Typed services";
    form.elements.owner.value = "Owner";
    form.elements.initial_safeguard.value = "Never weaken gates";
    form.elements.safeguard_scope.value = "Repository";
    form.elements.revision_triggers.value = "Mission changes";
    form.querySelector('button[type="submit"]').click();
  })()`);
  await waitFor('document.querySelector("#review-dialog").open', "exact review did not open");
  assert.equal(await cdp.evaluate('document.activeElement.id'), "review-cancel");
  assert.equal(await cdp.evaluate('document.querySelector("#review-record").textContent.includes("exact-browser-base")'), true);
  await cdp.send("Input.dispatchKeyEvent", {
    type: "rawKeyDown",
    key: "Escape",
    code: "Escape",
    windowsVirtualKeyCode: 27,
    nativeVirtualKeyCode: 27,
  });
  await cdp.send("Input.dispatchKeyEvent", {
    type: "keyUp",
    key: "Escape",
    code: "Escape",
    windowsVirtualKeyCode: 27,
    nativeVirtualKeyCode: 27,
  });
  await waitFor('!document.querySelector("#review-dialog").open', "escape did not cancel review");
  assert.equal(await cdp.evaluate('document.activeElement.textContent'), "Review changes");
  assert.equal(counters.agree, 0);

  await cdp.evaluate('document.querySelector("#init-form button[type=submit]").click()');
  await waitFor('document.querySelector("#review-dialog").open', "second exact review did not open");
  await cdp.evaluate('document.querySelector("#review-confirm").click()');
  await waitFor('document.querySelector("#command-record").textContent.includes("stale")', "stale response was not rendered");
  assert.equal(counters.agree, 1);

  const loadedAgain = cdp.event("Page.loadEventFired");
  await cdp.send("Page.reload");
  await loadedAgain;
  await waitFor('document.querySelector("#state").hidden', "reload did not recover");
  assert.equal(await cdp.evaluate('document.querySelector("#edit").hidden'), false);

  for (const width of [320, 390, 768, 1024]) {
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width,
      height: 900,
      deviceScaleFactor: 1,
      mobile: width < 768,
    });
    for (const tab of ["dashboard", "foundations", "enforcement", "decisions"]) {
      const overflow = await cdp.evaluate(`(() => {
        document.querySelector(${JSON.stringify(`#tab-${tab}`)}).click();
        return {
          tab: ${JSON.stringify(tab)},
          fits: document.documentElement.scrollWidth - window.innerWidth <= 5,
          viewport: window.innerWidth,
          documentWidth: document.documentElement.scrollWidth,
          offenders: [...document.querySelectorAll("body *")]
            .filter((node) => node.getBoundingClientRect().right > window.innerWidth + 1)
            .slice(0, 8)
            .map((node) => ({ tag: node.tagName, id: node.id, className: node.className, right: node.getBoundingClientRect().right })),
        };
      })()`);
      assert.equal(overflow.fits, true, `dashboard overflows at ${width}px: ${JSON.stringify(overflow)}`);
    }
  }

  await cdp.evaluate(`(() => {
    document.querySelector("#tab-decisions").click();
    const search = document.querySelector("#decision-search");
    search.value = "__empty__";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
  await waitFor('document.querySelector("#decision-history").textContent.includes("No decisions")', "empty state was not rendered");
  await cdp.evaluate('document.querySelector("#check-form button[type=submit]").click()');
  await waitFor('document.querySelector("#command-state").textContent === "command unknown"', "check result was not retained");
  await waitFor('document.querySelector("#decision-history").textContent.includes("No decisions")', "check refresh discarded the active history query");
  assert.equal(counters.lastInspect.search, "__empty__");
  await cdp.evaluate(`(() => {
    const search = document.querySelector("#decision-search");
    search.value = "__unknown__";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
  await waitFor('document.querySelector("#state").textContent === "unknown"', "unknown state was not rendered");
  assert.equal(await cdp.evaluate('document.querySelector("#state").hidden'), false);
  assert.equal(await cdp.evaluate('document.querySelector("#attention-title").textContent'), "Test your first safeguard");
  await cdp.evaluate(`(() => {
    const search = document.querySelector("#decision-search");
    search.value = "__draft__";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
  await waitFor('document.querySelector("#attention-title").textContent.includes("2 local draft")', "draft action was not surfaced");
  assert.equal(await cdp.evaluate('document.querySelector("#attention-continue").textContent'), "Open decisions");
  await cdp.evaluate('document.querySelector("#attention-continue").click()');
  assert.equal(await cdp.evaluate('document.querySelector("#tab-decisions").getAttribute("aria-selected")'), "true");
  await cdp.evaluate(`(() => {
    const search = document.querySelector("#decision-search");
    search.value = "__missing_current__";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
  await waitFor('document.querySelector("#attention-title").textContent === "Could not determine the next step"', "missing current state was not fail-closed");
  assert.equal(await cdp.evaluate('document.querySelector("#attention-continue").hidden'), true);
  await cdp.evaluate(`(() => {
    const search = document.querySelector("#decision-search");
    search.value = "__error__";
    search.dispatchEvent(new Event("input", { bubbles: true }));
  })()`);
  await waitFor('document.querySelector("#state").textContent === "unavailable"', "error state was not rendered");
  assert.equal(await cdp.evaluate('document.querySelector("#state").hidden'), false);
  assert.equal(await cdp.evaluate('document.querySelector("#attention-title").textContent'), "Could not determine the next step");
  assert.ok(counters.inspectQueries >= 3, "history controls did not use the typed inspection endpoint");

  console.log("PASS dashboard browser behavior at 320/390/768/1024 widths");
} finally {
  cdp?.socket.close();
  browser.kill("SIGTERM");
  await new Promise((resolveExit) => {
    if (browser.exitCode !== null) resolveExit();
    else browser.once("exit", resolveExit);
  });
  await new Promise((resolveClose) => server.close(resolveClose));
  rmSync(profile, { recursive: true, force: true });
}

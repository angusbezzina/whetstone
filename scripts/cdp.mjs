// Minimal Chrome DevTools Protocol client shared by the dashboard browser
// harnesses. Node 22+ (global WebSocket and fetch); no dependencies.

import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";

export const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export function findChrome() {
  return [
    process.env.CHROME_BIN,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ].filter(Boolean).find(existsSync);
}

class Cdp {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.sequence = 0;
    this.pending = new Map();
    this.errors = [];
    this.requests = [];
  }

  open() {
    return new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", reject, { once: true });
      this.socket.addEventListener("message", ({ data }) => {
        const message = JSON.parse(data);
        if (message.id && this.pending.has(message.id)) {
          const entry = this.pending.get(message.id);
          this.pending.delete(message.id);
          if (message.error) entry.reject(new Error(JSON.stringify(message.error)));
          else entry.resolve(message.result);
          return;
        }
        if (message.method === "Runtime.exceptionThrown") {
          this.errors.push(message.params.exceptionDetails?.exception?.description ?? "exception");
        } else if (message.method === "Runtime.consoleAPICalled" && message.params.type === "error") {
          this.errors.push(message.params.args.map((arg) => arg.value ?? arg.description).join(" "));
        } else if (message.method === "Log.entryAdded" && message.params.entry.level === "error") {
          this.errors.push(message.params.entry.text + " " + (message.params.entry.url ?? ""));
        } else if (message.method === "Network.requestWillBeSent") {
          this.requests.push(message.params.request.url);
        }
      });
    });
  }

  send(method, params = {}) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate(expression) {
    const result = await this.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    }
    return result.result.value;
  }

  async waitFor(expression, message, timeout = 8_000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      if (await this.evaluate(expression)) return;
      await delay(40);
    }
    const debug = await this.evaluate(`({ href: location.href, text: document.body?.innerText?.slice(0, 600) })`);
    throw new Error(`${message}\n${JSON.stringify(debug, null, 2)}`);
  }

  async navigate(url) {
    // A hash-only change would not reload the document; start from a blank page.
    await this.send("Page.navigate", { url: "about:blank" });
    await this.send("Page.navigate", { url });
    await this.waitFor("document.readyState === 'complete'", `navigation to ${url} did not complete`, 15_000);
  }

  async viewport(width, height = 900) {
    await this.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: width < 768 });
    await delay(120);
  }
}

export async function launch() {
  const chrome = findChrome();
  if (!chrome) return null;
  const profile = mkdtempSync(join(tmpdir(), "whetstone-cdp-"));
  const child = spawn(chrome, [
    "--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--no-first-run", "--no-default-browser-check", "--disable-background-networking",
    "--disable-dev-shm-usage", "--disable-gpu", "--no-sandbox", "--hide-scrollbars", "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  const portFile = join(profile, "DevToolsActivePort");
  for (let attempt = 0; attempt < 400 && !existsSync(portFile); attempt += 1) {
    if (child.exitCode !== null) throw new Error(`Chrome exited early with ${child.exitCode}`);
    await delay(25);
  }
  const port = readFileSync(portFile, "utf8").split("\n")[0];
  let page;
  for (let attempt = 0; attempt < 400 && !page; attempt += 1) {
    try {
      const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then((response) => response.json());
      page = targets.find((target) => target.type === "page" && !target.url.startsWith("chrome-extension:"));
    } catch {}
    if (!page) await delay(25);
  }
  if (!page) throw new Error("Chrome did not expose a page target");
  const cdp = new Cdp(page.webSocketDebuggerUrl);
  await cdp.open();
  await Promise.all([cdp.send("Page.enable"), cdp.send("Runtime.enable"), cdp.send("Log.enable"), cdp.send("Network.enable")]);
  return {
    cdp,
    // Chrome can still be flushing its profile after SIGKILL; wait for the
    // exit and retry the removal instead of racing it.
    async close() {
      if (child.exitCode === null && child.signalCode === null) {
        const exited = new Promise((resolveExit) => child.once("exit", resolveExit));
        child.kill("SIGKILL");
        await Promise.race([exited, delay(2000)]);
      }
      rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 50 });
    },
  };
}

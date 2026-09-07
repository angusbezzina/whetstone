"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const { chromium } = require(
  process.env.WHETSTONE_PLAYWRIGHT_MODULE || "playwright",
);

async function main() {
  const output = await fs.mkdtemp(
    path.join(os.tmpdir(), "whetstone-direction-"),
  );
  console.log(`Browser evidence: ${output}`);
  const browser = await chromium.launch({
    headless: true,
    channel: process.env.WHETSTONE_BROWSER_CHANNEL || "chrome",
  });
  const errors = [];
  const externalRequests = [];
  let assertions = 0;
  const check = (condition, message) => {
    assert.ok(condition, message);
    assertions += 1;
  };

  try {
    const context = await browser.newContext({
      viewport: { width: 1440, height: 1080 },
      reducedMotion: "reduce",
      acceptDownloads: true,
    });
    const page = await context.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("request", (request) => {
      if (/^https?:/.test(request.url())) externalRequests.push(request.url());
    });
    await page.goto(pathToFileURL(path.join(__dirname, "index.html")).href);
    check(
      (await page.title()).includes("Product direction"),
      "Default view is the brief",
    );
    check(await page.locator("#brief").isVisible(), "Brief is visible");
    await page.screenshot({
      path: path.join(output, "01-one-pager.png"),
      fullPage: true,
    });

    const navigate = async (name) => {
      await page.locator(`[data-nav="${name}"]`).click();
      await page.locator(`#${name}`).waitFor({ state: "visible" });
      check(await page.locator(`#${name}`).isVisible(), `${name} view opens`);
      check(
        (await page
          .locator(`[data-nav="${name}"]`)
          .getAttribute("aria-current")) === "page",
        `${name} has active navigation`,
      );
    };

    await navigate("workspace");
    check(
      (await page.locator("#rule-rows tr").count()) === 6,
      "Six sample standards",
    );
    await page.screenshot({
      path: path.join(output, "02-workspace.png"),
      fullPage: true,
    });
    for (const [filter, count, id] of [
      ["automated", 4, "ARCH-01"],
      ["human", 1, "PROC-05"],
      ["advisory", 1, "GUIDE-06"],
    ]) {
      await page.locator(`[data-filter="${filter}"]`).click();
      check(
        (await page.locator("#rule-rows tr").count()) === count,
        `${filter} filter count`,
      );
      check(
        (await page.locator("#inspector").innerText()).includes(id),
        `${filter} updates inspector`,
      );
    }
    await page.locator('[data-filter="all"]').click();
    for (const id of [
      "ARCH-01",
      "UI-02",
      "TEST-03",
      "PERF-04",
      "PROC-05",
      "GUIDE-06",
    ]) {
      await page.locator(`[data-standard="${id}"]`).click();
      check(
        (await page.locator("#inspector").innerText()).includes(id),
        `${id} details available`,
      );
    }
    await page.locator('[data-standard="ARCH-01"]').click();
    await page.locator("#inspector [data-decision]").click();
    check(
      await page.locator("#decision-dialog").isVisible(),
      "Decision dialog opens from standard",
    );
    check(
      (await page.locator("#dialog-title").innerText()).includes("D-002"),
      "Correct linked decision",
    );
    await page.keyboard.press("Escape");
    check(
      !(await page.locator("#decision-dialog").isVisible()),
      "Escape dismisses dialog",
    );
    await page.locator("#run-checks").click();
    await page
      .locator("#check-output")
      .getByText("MERGE: BLOCKED", { exact: true })
      .waitFor();
    const receipt = await page.locator("#check-output").innerText();
    for (const text of [
      "MERGE: BLOCKED",
      "DEPLOY: BLOCKED",
      "UNKNOWN",
      "PENDING",
      "NO COMMAND EXECUTED",
    ]) {
      check(receipt.includes(text), `Receipt preserves ${text}`);
    }

    await navigate("setup");
    await page.locator("#setup-next").click();
    const mission =
      'Ship "reliable" changes <without> hidden trade-offs & surprises.';
    await page.locator("#setup-mission").fill(mission);
    await page.locator("#setup-owner").fill("Architecture maintainers");
    await page
      .locator("#setup-philosophy")
      .selectOption("Team-defined mixed approach");
    await page.locator("#setup-metric").fill("Checkout p95 ≤ 220 ms");
    await page.locator("#setup-next").click();
    check(
      (await page.locator("#setup-panel").innerText()).includes(
        "Use the tools",
      ),
      "Wiring preview opens",
    );
    await page.locator("#setup-back").click();
    check(
      (await page.locator("#setup-mission").inputValue()) === mission,
      "Draft survives backward navigation",
    );
    check(
      (await page.locator("#setup-owner").inputValue()) ===
        "Architecture maintainers",
      "Owner draft preserved",
    );
    await page.locator("#setup-next").click();
    await page.locator("#setup-next").click();
    check(
      (await page.locator(".proposal-summary").innerText()).includes(mission),
      "User input is displayed literally, not HTML",
    );
    check(
      (await page.locator(".proposal-summary").innerText()).includes("No / No"),
      "Setup does not imply approval or installation",
    );
    const downloaded = page.waitForEvent("download");
    await page.locator("#download-proposal").click();
    const download = await downloaded;
    const proposalPath = path.join(output, download.suggestedFilename());
    await download.saveAs(proposalPath);
    const proposal = JSON.parse(await fs.readFile(proposalPath, "utf8"));
    check(
      proposal.approved === false && proposal.installed === false,
      "Download is an unapproved, uninstalled proposal",
    );
    check(
      proposal.agreement.mission === mission,
      "Download preserves edited mission",
    );
    check(
      proposal.agreement.philosophy === "Team-defined mixed approach",
      "Download preserves philosophy",
    );
    check(
      proposal.standards.every(
        (rule) => rule.status === "candidate" && rule.evidence === "not-run",
      ),
      "Proposed checks never inherit sample passing receipts",
    );
    await page.screenshot({
      path: path.join(output, "03-setup.png"),
      fullPage: true,
    });

    await navigate("workflows");
    for (const flow of ["human", "agent", "evolve"]) {
      await page.locator(`[data-flow="${flow}"]`).click();
      check(
        (await page.locator("#flow-steps button").count()) === 4,
        `${flow} has four steps`,
      );
      for (let step = 0; step < 4; step += 1) {
        await page.locator(`[data-flow-step="${step}"]`).click();
        check(
          (await page.locator("#flow-terminal-title").innerText()).includes(
            `STEP 0${step + 1}`,
          ),
          `${flow} step ${step + 1} updates command`,
        );
        check(
          (await page.locator("#flow-terminal").innerText()).includes(
            "no execution",
          ),
          `${flow} step ${step + 1} has simulation label`,
        );
        check(
          (await page.locator("#flow-boundary").innerText()).length > 30,
          `${flow} step ${step + 1} has authority boundary`,
        );
      }
    }
    await page.locator('[data-flow="agent"]').click();
    await page.screenshot({
      path: path.join(output, "04-agent-flow.png"),
      fullPage: true,
    });
    await page.evaluate(() =>
      Object.defineProperty(navigator, "clipboard", {
        value: undefined,
        configurable: true,
      }),
    );
    await page.locator("#copy-command").click();
    check(
      (await page.locator("#toast").innerText()).includes("manual copying"),
      "Clipboard fallback is explicit",
    );
    check(
      (await page.evaluate(() => window.getSelection().toString())).includes(
        "wh context",
      ),
      "Clipboard fallback selects the example",
    );
    await page.locator(".command-reference summary").click();
    check(
      (await page.locator(".command-table").innerText()).includes(
        "EXISTS IN v0.12",
      ),
      "Reference distinguishes existing commands",
    );

    await navigate("decisions");
    check(
      (await page.locator(".decision-card").count()) === 6,
      "Full history is available",
    );
    check(
      (await page.locator(".decision-card").first().innerText()).includes(
        "D-006",
      ),
      "Newest-first default",
    );
    check(
      (
        await page
          .locator(".decision-card")
          .filter({ hasText: "D-003 /" })
          .innerText()
      ).includes("Superseded"),
      "Current history preserves superseded decision",
    );
    await page.locator("#history-asof").selectOption("2026-03-31");
    check(
      (await page.locator(".decision-card").count()) === 3,
      "Historical cutoff excludes future decisions",
    );
    check(
      (await page.locator("#asof-note").innerText()).includes("400 ms"),
      "Historical metric target is preserved",
    );
    const originalTarget = page
      .locator(".decision-card")
      .filter({ hasText: "D-003 /" });
    check(
      (await originalTarget.innerText()).includes("Accepted") &&
        !(await originalTarget.innerText()).includes("Superseded"),
      "D-003 was accepted before its replacement",
    );
    await originalTarget.locator("[data-decision]").click();
    check(
      (await page.locator("#dialog-body").innerText()).includes("31 Mar 2026"),
      "Dialog uses historical cutoff",
    );
    await page.locator("#close-dialog").click();
    await page.locator("#history-asof").selectOption("2026-01-12");
    check(
      (await page.locator(".decision-card").count()) === 1,
      "Project-inception view exists",
    );
    check(
      (await page.locator(".decision-card").innerText()).includes("D-001"),
      "First project decision retained",
    );
    await page.locator("#history-asof").selectOption("2026-09-07");
    await page.locator("#decision-search").fill("no-such-decision");
    check(
      await page.locator(".empty").isVisible(),
      "Empty search has explanation",
    );
    await page.locator("#decision-search").fill("functional core");
    check(
      (await page.locator(".decision-card").count()) === 1,
      "Rationale search finds matching record",
    );
    await page.locator("#decision-search").fill("");
    await page.locator("#history-order").click();
    check(
      (await page.locator(".decision-card").first().innerText()).includes(
        "D-001",
      ),
      "Oldest-first begins at inception",
    );
    await page.screenshot({
      path: path.join(output, "05-history.png"),
      fullPage: true,
    });

    for (const width of [320, 390, 768, 1024]) {
      await page.setViewportSize({ width, height: 900 });
      for (const view of [
        "brief",
        "workspace",
        "setup",
        "workflows",
        "decisions",
      ]) {
        await navigate(view);
        const dimensions = await page.evaluate(() => ({
          scroll: document.documentElement.scrollWidth,
          viewport: innerWidth,
        }));
        if (dimensions.scroll > dimensions.viewport) {
          console.log(
            await page.evaluate(() =>
              Array.from(document.querySelectorAll("body *"))
                .filter((element) => {
                  const rect = element.getBoundingClientRect();
                  return rect.width && rect.right > innerWidth;
                })
                .map((element) => ({
                  tag: element.tagName,
                  class: element.className,
                  right: element.getBoundingClientRect().right,
                  text: element.textContent.slice(0, 80),
                }))
                .slice(0, 15),
            ),
          );
          await page.screenshot({
            path: path.join(output, `overflow-${view}-${width}.png`),
            fullPage: true,
          });
        }
        check(
          dimensions.scroll <= dimensions.viewport,
          `No horizontal page overflow: ${view} at ${width}px`,
        );
      }
      if (width === 390) {
        await navigate("workspace");
        await page.screenshot({
          path: path.join(output, "06-mobile-workspace.png"),
          fullPage: true,
        });
      }
    }

    await page.setViewportSize({ width: 1440, height: 1080 });
    await page.goto(pathToFileURL(path.join(__dirname, "index.html")).href);
    await page.keyboard.press("Tab");
    check(
      await page
        .locator(".skip")
        .evaluate((element) => element === document.activeElement),
      "Skip link is first keyboard target",
    );
    await page.keyboard.press("Enter");
    check(
      await page
        .locator("#main")
        .evaluate((element) => element === document.activeElement),
      "Skip link reaches main content",
    );
    await navigate("workspace");
    await page.emulateMedia({ media: "print" });
    check(
      await page.locator("#brief").isVisible(),
      "Print always exposes the brief",
    );
    check(
      !(await page.locator("#workspace").isVisible()),
      "Print omits interactive views",
    );
    const pdf = await page.pdf({
      path: path.join(output, "whetstone-one-pager.pdf"),
      preferCSSPageSize: true,
      printBackground: true,
    });
    const pageCount = (pdf.toString("latin1").match(/\/Type\s*\/Page\b/g) || [])
      .length;
    check(
      pageCount === 1,
      `One-pager prints on exactly one page (got ${pageCount})`,
    );
    check(errors.length === 0, `No browser errors: ${errors.join("; ")}`);
    check(
      externalRequests.length === 0,
      "The HTML works without external requests",
    );
    console.log(
      JSON.stringify(
        {
          status: "passed",
          assertions,
          browserErrors: errors,
          externalRequests,
          output,
        },
        null,
        2,
      ),
    );
  } finally {
    await browser.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});

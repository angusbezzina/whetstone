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
    const htmlSource = await fs.readFile(
      path.join(__dirname, "index.html"),
      "utf8",
    );
    const allowedCommands = ["init", "dash", "change", "check", "push", "pull"];
    const commandNames = [
      ...new Set(
        Array.from(
          htmlSource.matchAll(/\bwh (init|dash|change|check|push|pull)\b/g),
          (match) => match[1],
        ),
      ),
    ];
    check(
      !/\bwh (setup|context|verify|decisions?|ui|extract|rules|actions|scan|status|mcp)\b/.test(
        htmlSource,
      ),
      "The previous command tree is removed",
    );
    check(
      allowedCommands.every((command) => commandNames.includes(command)),
      "Every agreed command appears in the artefact",
    );
    check(
      !htmlSource.includes("stored in Git") &&
        !htmlSource.includes("mandatory database"),
      "The previous Git-authoritative model is removed",
    );
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
    check(
      (await page.locator(".command-strip code").count()) === 6,
      "One-pager exposes six commands",
    );
    check(
      (await page.locator("#brief").innerText()).includes("Dolt"),
      "One-pager explains Dolt-backed state",
    );
    const brief = await page.locator("#brief").innerText();
    check(
      brief.includes("Check → repair → recheck") &&
        brief.includes("Decide → observe → improve") &&
        brief.includes("Verified ≠ authorized ≠ a healthy outcome") &&
        !brief.includes("Evidence you can inspect"),
      "One-pager connects correction to accountable decisions and outcomes",
    );
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
    check(
      (await page.locator("#loop-scenario").inputValue()) === "proactive",
      "Workspace leads with approved signal-triggered work",
    );
    check(
      !(await page.locator(".loop-evidence").getAttribute("open")),
      "Agent receipts are collapsed by default behind a human summary",
    );
    check(
      (await page.locator("#loop-scenario option").count()) === 8,
      "Eight cases fit the existing workspace without more navigation",
    );
    await page.screenshot({
      path: path.join(output, "02-workspace.png"),
      fullPage: true,
    });
    const referenceBefore = await page.locator("#rule-rows").innerText();
    const sharingBefore = await page.locator("#sharing-status").innerText();
    check(
      (await page.locator(".reference-label").innerText()).includes(
        "not the task replay",
      ),
      "Task simulation is explicitly separate from the reference PR",
    );
    for (const [scenario, task, states] of [
      [
        "proactive",
        "M-047",
        [
          "Signal detected — no task yet",
          "Reproduced — work authorized",
          "Repair verified — release pending",
          "Improvement observed — review due",
        ],
      ],
      [
        "promotion",
        "P-048",
        [
          "Local experiment — team unchanged",
          "Shared — independent review pending",
          "Accepted — activation pending",
          "Activated — impact pending",
        ],
      ],
      [
        "cleanup",
        "C-049",
        [
          "Stale context — investigation permitted",
          "Archive proposed — review pending",
          "Reviewed — archive pending",
          "Archived — history retained",
        ],
      ],
      [
        "routine",
        "T-042",
        ["Working", "Repair needed", "Recheck required", "Task verified"],
      ],
      [
        "owner",
        "T-043",
        ["Working", "Owner required", "Awaiting decision", "Blocked — owner"],
      ],
      [
        "unavailable",
        "T-044",
        [
          "Working",
          "Checker unavailable",
          "Retry budget reached",
          "Blocked — verification",
        ],
      ],
      [
        "redirect",
        "T-045",
        [
          "Verified — decision needed",
          "Redirected — narrower scope",
          "Reworked — recheck required",
          "Reverified — acceptance pending",
        ],
      ],
      [
        "outcome",
        "T-046",
        [
          "Accepted — observation pending",
          "Outcome breach — follow-up open",
          "Repair candidate — recheck required",
          "Repair verified — follow-through open",
        ],
      ],
    ]) {
      await page.locator("#loop-scenario").selectOption(scenario);
      check(
        (await page.locator("#loop-task-caption").innerText()).includes(task),
        `${scenario} uses its own task identity`,
      );
      for (let step = 0; step < 4; step += 1) {
        check(
          !(await page.locator(".loop-evidence").getAttribute("open")),
          `${scenario} stage ${step + 1} starts with readable summary, not terminal output`,
        );
        await page.locator(".loop-evidence summary").click();
        check(
          (await page.locator("#loop-status").innerText()) === states[step],
          `${scenario} step ${step + 1} exposes the correct state`,
        );
        check(
          (
            await page.locator('#loop-steps [aria-current="step"]').innerText()
          ).startsWith(`${step + 1}.`),
          `${scenario} announces the current stage`,
        );
        const result = await page.locator("#loop-result").innerText();
        check(
          result.includes("NO COMMAND EXECUTED"),
          `${scenario} is clearly a simulation`,
        );
        check(
          (await page.locator(".loop-axes dt").allTextContents()).join("|") ===
            "Verification|Authorization|Outcome",
          `${scenario} step ${step + 1} separates all three completion axes`,
        );
        for (const axis of ["verification", "authorization", "outcome"]) {
          check(
            (await page.locator(`[data-axis="${axis}"]`).innerText()).length >
              10,
            `${scenario} step ${step + 1} exposes a meaningful ${axis} state`,
          );
        }
        const additions = {
          proactive: [
            [
              "approved before this observation",
              "No task exists yet",
              "30 runs",
              "freshness limit",
              "no payments",
              "not a new scheduler",
            ],
            [
              "reproduces",
              "dedup key",
              "no second task or worker",
              "one worker",
              "30-minute cooldown",
              "2 attempts / 20 minutes",
              "revocation",
              "before each side effect",
            ],
            [
              "abc4702",
              "95 ms",
              "safeguards unchanged",
              "not merge or release",
              "no automatic deployment",
              "live result unknown",
            ],
            [
              "separately authorized",
              "1,400",
              "causality not proven",
              "Owner:",
              "No effect",
              "outcome unresolved",
              "no automatic policy learning",
            ],
          ],
          promotion: [
            [
              "250 ms",
              "220 ms",
              "reported separately",
              "not strengthen it",
              "Private notes",
              "not accepted team authority",
            ],
            [
              "principal rowan",
              "principal priya",
              "Self-approval: rejected",
              "Solo mode",
              "No private record IDs",
              "base v1.3",
            ],
            [
              "pkg0481",
              "Accepted v1.4 / required v1.3 / installed v1.3",
              "Stale response: rejected",
              "duplicate approval: idempotent",
              "scripts remain inert",
            ],
            [
              "checks-v4",
              "Known-bad, known-good",
              "Runner B: v1.3 stale",
              "behavior may differ",
              "next 7 days",
              "separate PR #184",
            ],
          ],
          cleanup: [
            [
              "completion event",
              "no deletion",
              "Overlap is a hypothesis",
              "Age alone",
            ],
            [
              "content hash",
              "Reference audit",
              "searchable and recoverable",
              "invalidates this proposal",
            ],
            [
              "current source hash",
              "recovery path",
              "verify archive retrieval",
              "keep original active source intact",
            ],
            [
              "doc0491 retrievable",
              "superseded plan excluded",
              "as-of retrieval retains it",
              "No broken links",
              "restore by a new reviewed change",
              "no files were archived",
            ],
          ],
        };
        if (additions[scenario]) {
          for (const token of additions[scenario][step])
            check(
              result.toLowerCase().includes(token.toLowerCase()),
              `${scenario} stage ${step + 1} preserves ${token}`,
            );
          if (step === 3) {
            await page.locator(".loop-evidence summary").click();
            await page.evaluate(() => window.scrollTo(0, 0));
            await page.screenshot({
              path: path.join(output, `15-${scenario}.png`),
              fullPage: true,
            });
          }
        }
        if (scenario === "routine" && step === 1) {
          for (const token of [
            "ARCH-01",
            "retry.ts:18",
            "Why:",
            "Next:",
            "Verify:",
            "c0ffee1",
          ]) {
            check(result.includes(token), `Feedback is actionable: ${token}`);
          }
          await page.screenshot({
            path: path.join(output, "09-feedback.png"),
            fullPage: true,
          });
        }
        if (scenario === "routine" && step === 2) {
          check(
            result.includes("c0ffee2") &&
              result.includes("invalid for c0ffee2"),
            "Repair invalidates the old receipt",
          );
          check(
            result.includes("Policy, checker, tests, baselines: unchanged"),
            "Repair does not weaken safeguards",
          );
          check(
            !result.includes("Task result: verified"),
            "An edit is not premature completion",
          );
        }
        if (scenario === "routine" && step === 3) {
          for (const token of [
            "code c0ffee2",
            "policy v1.3",
            "checks-v3",
            "0 owner interruptions",
            "not triggered",
            "production health remains unknown",
          ]) {
            check(
              result.includes(token),
              `Final verification preserves ${token}`,
            );
          }
          check(
            result.includes("same code/policy/check snapshot"),
            "Final required CI checks are bound to the repaired snapshot",
          );
          await page.screenshot({
            path: path.join(output, "10-verified-repair.png"),
            fullPage: true,
          });
        }
        if (scenario === "owner" && step === 2) {
          for (const token of [
            "Decision needed:",
            "Why you:",
            "Recommended:",
            "Alternative:",
            "Impact:",
            "Evidence:",
            "Review target:",
          ]) {
            check(result.includes(token), `Owner package includes ${token}`);
          }
          await page.screenshot({
            path: path.join(output, "11-owner-decision.png"),
            fullPage: true,
          });
        }
        if (scenario === "owner" && step === 3) {
          check(
            result.includes("expected current revision 3") &&
              result.includes("Stale response: reject"),
            "Owner handoff is revision-safe",
          );
          check(
            result.includes("NOT COMPLETE"),
            "Missing owner authority blocks completion",
          );
        }
        if (scenario === "unavailable" && step === 3) {
          check(
            result.includes("Attempts: 2") &&
              result.includes("NOT COMPLETE") &&
              result.includes("result: unknown"),
            "Checker failure has bounded retries and cannot pass",
          );
          check(
            result.includes("expected current revision 4") &&
              result.includes("code c0ffee3") &&
              result.includes("required policy v1.3") &&
              result.includes("Platform environment maintainer"),
            "Unavailable-checker handoff pins its revision, snapshot, policy, and responsible owner",
          );
          await page.screenshot({
            path: path.join(output, "12-checker-unavailable.png"),
            fullPage: true,
          });
        }
        if (scenario === "redirect") {
          const required = [
            [
              "Passed · abc4501",
              "New service not authorized",
              "existing queue",
              "Platform owners",
              "expected task revision 1",
            ],
            [
              "new service rejected despite passing checks",
              "operational simplicity",
              "2 repair attempts / 20 minutes",
              "Stale reply",
              "local rework only",
            ],
            [
              "V-045.1 is invalid for abc4502",
              "not acceptance or release",
              "Existing",
              "wh check",
            ],
            [
              "V-045.2",
              "Acceptance: PENDING",
              "expected",
              "NOT COMPLETE",
              "No automatic merge, deployment, or publication",
            ],
          ];
          // Rework explicitly preserves existing safeguards, regardless of prose case.
          for (const token of required[step])
            check(
              result.toLowerCase().includes(token.toLowerCase()),
              `Redirect stage ${step + 1} preserves ${token}`,
            );
          check(
            !(await page.locator('[data-axis="outcome"]').innerText()).includes(
              "healthy",
            ),
            "An undeployed candidate cannot claim a healthy outcome",
          );
          if (step === 1) {
            await page.evaluate(() => window.scrollTo(0, 0));
            await page.screenshot({
              path: path.join(output, "13-verified-but-redirected.png"),
              fullPage: true,
            });
          }
        }
        if (scenario === "outcome") {
          const required = [
            [
              "D-046.1",
              "separately authorized release",
              "Expected:",
              "O-046.1",
              "Window:",
              "Freshness:",
              "Owner:",
              "Recovery:",
              "no production writes",
            ],
            [
              "1,240",
              "3 confirmed duplicate charges",
              "Causality: not yet established",
              "INC-046",
              "Payments on-call",
              "Next for agent:",
              "2 attempts / 20 min",
            ],
            [
              "redacted fixture",
              "do not cover abc4602",
              "no silent promotion",
              "Incident INC-046 remains open",
              "wh change",
            ],
            [
              "V-046.2",
              "Acceptance / release of abc4602: PENDING",
              "D-046.1 cannot authorize",
              "expected task revision 4",
              "fresh 24h observation",
              "live outcome unresolved",
              "no automatic deployment or wh push",
            ],
          ];
          for (const token of required[step])
            check(
              result.includes(token),
              `Outcome stage ${step + 1} preserves ${token}`,
            );
          if (step === 3) {
            check(
              (
                await page.locator('[data-axis="verification"]').innerText()
              ).startsWith("Passed"),
              "Repair verification can pass",
            );
            check(
              (
                await page.locator('[data-axis="authorization"]').innerText()
              ).includes("pending"),
              "Repair verification cannot authorize its own release",
            );
            check(
              (
                await page.locator('[data-axis="outcome"]').innerText()
              ).startsWith("Unresolved"),
              "Local repair cannot close the operational outcome",
            );
            await page.evaluate(() => window.scrollTo(0, 0));
            await page.screenshot({
              path: path.join(output, "14-outcome-follow-through.png"),
              fullPage: true,
            });
          }
        }
        if (step < 3) await page.locator("#loop-next").click();
      }
      check(
        await page.locator("#loop-next").isDisabled(),
        `${scenario} stops at its terminal state`,
      );
      check(
        (await page.locator("#rule-rows").innerText()) === referenceBefore,
        `${scenario} does not change unrelated reference gates`,
      );
      check(
        (await page.locator("#sharing-status").innerText()) === sharingBefore,
        `${scenario} does not publish or approve policy`,
      );
      await page.locator("#loop-reset").click();
      check(
        (await page.locator("#loop-status").innerText()) === states[0],
        `${scenario} restart clears its result`,
      );
      check(
        !(await page.locator("#loop-next").isDisabled()),
        `${scenario} can be replayed`,
      );
    }
    for (const scenario of ["proactive", "promotion", "outcome"]) {
      await page.locator(`[data-case="${scenario}"]`).click();
      check(
        (await page.locator("#loop-scenario").inputValue()) === scenario,
        `${scenario} shortcut opens the correct case`,
      );
      check(
        (
          await page.locator('#loop-steps [aria-current="step"]').innerText()
        ).startsWith("1."),
        "Case shortcut starts a fresh replay",
      );
      await page.locator("#loop-next").click();
    }
    await page.locator("#loop-scenario").selectOption("routine");
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
      check(
        !(await page.locator(".applicability").getAttribute("open")),
        `${id} applicability is expandable`,
      );
      await page.locator(".applicability summary").click();
      const relevance = await page.locator(".applicability").innerText();
      for (const token of [
        "Applies because:",
        "Derived from:",
        "Evidence or judgment:",
        "Mandatory checks cannot be omitted",
        id,
        "ctx12 stale",
        "not guaranteed identical",
      ])
        check(relevance.includes(token), `${id} explains ${token}`);
      if (id === "TEST-03")
        check(
          relevance.includes("cannot prove test-first development"),
          "Test results are not fake process evidence",
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

    check(
      (await page.locator("#sharing-status").innerText()).includes(
        "2 local drafts",
      ),
      "Running checks does not publish local drafts",
    );
    await page.locator("#preview-push").click();
    check(await page.locator("#push-dialog").isVisible(), "Push preview opens");
    check(
      await page.locator("#confirm-push").isDisabled(),
      "Sharing requires an explicit selection",
    );
    check(
      (await page.locator("[data-share-id]").count()) === 2,
      "Only shareable drafts can be selected",
    );
    check(
      (await page.locator('[data-share-id="P-001"]').count()) === 0,
      "Private preference is not selectable",
    );
    check(
      (await page.locator("#push-body").innerText()).includes(
        "Private records and their history",
      ),
      "Privacy includes unpublished history",
    );
    await page.locator('[data-share-id="L-007"]').check();
    let payload = JSON.parse(await page.locator("#push-payload").innerText());
    check(
      payload.changes.length === 1 && payload.changes[0].id === "L-007",
      "Preview includes exactly the selected draft",
    );
    check(
      payload.destination === "team / northstar-policies (fictional)",
      "Sharing preview explicitly identifies its destination",
    );
    check(
      !JSON.stringify(payload).includes("P-001") &&
        !JSON.stringify(payload).includes("explicit exports"),
      "Outgoing payload excludes private identity and content",
    );
    await page.locator("#cancel-push").click();
    check(
      (await page.locator("#sharing-status").innerText()).includes(
        "2 local drafts",
      ),
      "Cancelling a preview leaves all drafts local",
    );
    await page.locator("#preview-push").click();
    await page.screenshot({
      path: path.join(output, "07-push-preview.png"),
      fullPage: true,
    });
    await page.locator("#confirm-push").click();
    check(
      (await page.locator("#push-body").innerText()).includes(
        "Approval is pending",
      ),
      "Publishing does not grant approval",
    );
    payload = JSON.parse(await page.locator("#push-payload").innerText());
    check(
      payload.approval === "pending" && payload.baseAgreement === "v1.3",
      "Publication pins the base agreement and remains a proposal",
    );
    check(
      payload.destination === "team / northstar-policies (fictional)",
      "Confirmed publication preserves the reviewed destination",
    );
    check(
      payload.changes.length === 1 && payload.changes[0].id === "L-007",
      "Confirmed publication matches the preview",
    );
    await page.keyboard.press("Escape");
    check(
      !(await page.locator("#push-dialog").isVisible()),
      "Sharing dialog dismisses with Escape",
    );
    const sharedStatus = await page.locator("#sharing-status").innerText();
    check(
      sharedStatus.includes("1 local draft") &&
        sharedStatus.includes("1 shared proposal") &&
        sharedStatus.includes("1 private preference"),
      "Unselected and private records remain local",
    );
    check(
      sharedStatus.includes("team v1.3 unchanged"),
      "Publishing does not replace the accepted team agreement",
    );
    check(
      (await page.locator("#rule-rows tr").count()) === 6,
      "Shared proposals are not inserted as accepted standards",
    );
    await page.locator("#demo-pull").click();
    const pullReceipt = await page.locator("#sync-receipt").innerText();
    check(
      pullReceipt.includes("conflict") &&
        pullReceipt.includes("220 ms") &&
        pullReceipt.includes("225 ms"),
      "Pull surfaces both sides of a conflicting change",
    );
    check(
      pullReceipt.includes("preserved") &&
        pullReceipt.includes("Accepted: v1.4") &&
        pullReceipt.includes("Required for this release: v1.3") &&
        pullReceipt.includes("Installed: v1.3"),
      "Pull preserves drafts and distinguishes accepted, required, and installed policy",
    );
    check(
      pullReceipt.includes("old v1.3 checks cannot establish compliance"),
      "Local conflicts cannot downgrade future required team policy",
    );
    check(
      (await page.locator("#sharing-status").innerText()) === sharedStatus,
      "Pull does not publish pending local changes",
    );
    await page.locator("#demo-pull").click();
    check(
      (await page.locator("#sync-receipt").innerText()) === pullReceipt,
      "Repeated pull is a stable replay",
    );
    await page.locator("#preview-push").click();
    check(
      await page.locator('[data-share-id="L-007"]').isDisabled(),
      "Already shared changes cannot be accidentally resent",
    );
    check(
      await page.locator("#confirm-push").isDisabled(),
      "A subsequent push requires another explicit selection",
    );
    check(
      !(await page.locator('[data-share-id="L-008"]').isDisabled()),
      "The unshared draft is still available locally",
    );
    await page.locator("#close-push").click();
    await page.screenshot({
      path: path.join(output, "08-local-and-shared.png"),
      fullPage: true,
    });

    await navigate("setup");
    await page.locator("#setup-next").click();
    const mission =
      'Ship "reliable" changes <without> hidden trade-offs & surprises.';
    await page.locator("#setup-mission").fill(mission);
    await page.locator("#setup-owner").fill("Architecture maintainers");
    await page
      .locator("#setup-values")
      .fill("Reliability & explicit ownership");
    await page
      .locator("#setup-outcome")
      .fill("More successful journeys without increased support burden");
    await page
      .locator("#setup-philosophy")
      .selectOption("Team-defined mixed approach");
    await page.locator("#setup-metric").fill("Checkout p95 ≤ 220 ms");
    check(
      !(await page.locator("#setup-delegation").getAttribute("open")),
      "Inherited defaults stay condensed until requested",
    );
    await page.locator("#setup-delegation summary").click();
    const allowed = 'Checkout edits <only> & existing "trusted" checks';
    await page.locator("#setup-allowed").fill(allowed);
    await page.locator("#setup-observationOwner").fill("Reliability on-call");
    const trigger = 'Trusted "jank" probe <checkout> & explicit mandate';
    await page.locator("#setup-trigger").fill(trigger);
    await page
      .locator("#setup-limits")
      .fill("2 attempts / 15 minutes; no new permissions");
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
    check(
      (await page.locator("#setup-allowed").inputValue()) === allowed,
      "Delegation defaults survive backward navigation and escaping",
    );
    check(
      (await page.locator("#setup-observationOwner").inputValue()) ===
        "Reliability on-call",
      "Follow-through owner survives backward navigation",
    );
    check(
      (await page.locator("#setup-trigger").inputValue()) === trigger,
      "Signal-triggered mandate preserves literal user input across navigation",
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
    await page.locator("#setup-defaults-summary summary").click();
    check(
      (await page.locator("#setup-defaults-summary").innerText()).includes(
        allowed,
      ),
      "Default summary displays user values literally",
    );
    const downloaded = page.waitForEvent("download");
    await page.locator("#download-proposal").click();
    const download = await downloaded;
    const proposalPath = path.join(output, download.suggestedFilename());
    await download.saveAs(proposalPath);
    const proposal = JSON.parse(await fs.readFile(proposalPath, "utf8"));
    check(
      proposal.published === false && proposal.visibility === "local-only",
      "Setup exports remain local and unpublished",
    );
    check(
      proposal.storage.proposedBackend === "dolt" &&
        proposal.storage.actualDemo === "in-memory",
      "Proposal distinguishes intended Dolt backend from this simulation",
    );
    check(
      proposal.approved === false && proposal.installed === false,
      "Download is an unapproved, uninstalled proposal",
    );
    check(
      proposal.authorityGranted === false &&
        proposal.outcomeMonitoringActive === false &&
        proposal.proactiveWorkEnabled === false,
      "Downloading defaults grants no authority and activates no monitoring",
    );
    check(
      proposal.agreement.trigger === trigger &&
        proposal.agreement.initiation.includes("one worker") &&
        proposal.agreement.reviewPolicy.includes("other than proposer") &&
        proposal.agreement.contextPolicy.includes("preserve full history"),
      "Export includes mandate, deduplication, independent review, and historical retention proposals",
    );
    check(
      (await page.locator("#setup-defaults-summary").innerText()).includes(
        trigger,
      ),
      "Final summary displays the unapproved mandate literally",
    );
    check(
      proposal.agreement.allowed === allowed &&
        proposal.agreement.limits.includes("15 minutes"),
      "Export preserves scoped delegation and limits",
    );
    check(
      proposal.agreement.observationOwner === "Reliability on-call" &&
        proposal.agreement.observationSource.includes("telemetry") &&
        proposal.agreement.observationWindow.includes("24h") &&
        proposal.agreement.recovery.includes("rollback"),
      "Export includes owner, source, window, and recovery path",
    );
    for (const token of [
      "standing authority",
      "adverse outcome",
      "valid alternatives",
      "model, skill",
      "signal-triggered mandate",
      "independent human review",
      "Explain applicability",
      "archival preserves references",
    ])
      check(
        proposal.reviewRequired.some((item) => item.includes(token)),
        `Setup requires review of ${token}`,
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
      proposal.agreement.values === "Reliability & explicit ownership",
      "Download preserves editable core values",
    );
    check(
      proposal.agreement.outcome.includes("successful journeys"),
      "Mission outcome is distinct from the safeguard",
    );
    check(
      proposal.reviewRequired.some((item) =>
        item.includes("known-good recheck"),
      ),
      "Setup proves correction, not just detection",
    );
    check(
      proposal.reviewRequired.some((item) =>
        item.includes("bounded escalation"),
      ),
      "Setup proves the blocked path as well",
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
        if (flow === "evolve" && step === 2)
          check(
            (await page.locator("#flow-terminal").innerText()).includes(
              "requires a different authorized human reviewer from its author",
            ),
            "A reviewer who authors a counterproposal cannot self-approve it",
          );
      }
    }
    await page.locator('[data-flow="agent"]').click();
    await page.locator(".workflow-contract summary").click();
    const contract = await page.locator(".workflow-contract").innerText();
    for (const token of [
      "needs_execution_approval",
      "stale answers are rejected",
      "private or unavailable dependency",
      "protected activation record",
      "experiments have separate results",
      "owner-approved scoped exception",
      "known-bad and known-good",
      "false blocks",
      "unsupported constraints are advisory",
      "standing code-work authority never authorizes wh push",
      "local repair cannot close a live incident",
      "stale approval or changed checker",
      "failed user requirement",
      "accepted and rejected preference examples",
      "unnecessary owner interruptions",
      "escaped failures",
      "overdue follow-through",
      "a mission explains why; a mandate grants permission",
      "recheck revocation before side effects",
      "wh check itself never starts a repair",
      "other than the proposer",
      "passing tests cannot prove test-first development",
      "before/after movement alone does not prove causality",
      "age or similarity alone never authorizes deletion",
      "historical as-of queries retain it",
      "beads can track work",
    ]) {
      check(
        contract
          .toLowerCase()
          .replace(/\s+/g, " ")
          .includes(token.toLowerCase()),
        `Operating contract includes ${token}`,
      );
    }
    await page.locator(".workflow-contract summary").click();
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
        "wh --json",
      ),
      "Clipboard fallback selects the example",
    );
    await page.locator(".command-reference summary").click();
    check(
      (await page.locator(".command-reference").innerText()).includes(
        "not implemented in v0.12",
      ),
      "Reference distinguishes the concept from shipped functionality",
    );
    const listedCommands = await page
      .locator(".command-table [data-command]")
      .evaluateAll((rows) => rows.map((row) => row.dataset.command));
    check(
      listedCommands.length === 6 &&
        allowedCommands.every((command) => listedCommands.includes(command)),
      "Reference has exactly six command families",
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
      await navigate("workspace");
      await page.locator("#loop-scenario").selectOption("owner");
      for (let step = 0; step < 3; step += 1)
        await page.locator("#loop-next").click();
      await page.locator(".loop-evidence summary").click();
      check(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
        `Long blocked handoff fits at ${width}px`,
      );
      for (const scenario of [
        "redirect",
        "outcome",
        "proactive",
        "promotion",
        "cleanup",
      ]) {
        await page.locator("#loop-scenario").selectOption(scenario);
        for (let step = 0; step < 4; step += 1) {
          check(
            await page.evaluate(
              () => document.documentElement.scrollWidth <= innerWidth,
            ),
            `${scenario} condensed stage ${step + 1} fits at ${width}px`,
          );
          await page.locator(".loop-evidence summary").click();
          check(
            await page.evaluate(
              () => document.documentElement.scrollWidth <= innerWidth,
            ),
            `${scenario} stage ${step + 1} fits at ${width}px`,
          );
          if (step < 3) await page.locator("#loop-next").click();
        }
      }
      await page.locator("#preview-push").click();
      check(
        await page
          .locator("#push-dialog")
          .evaluate(
            (dialog) =>
              dialog.getBoundingClientRect().right <= innerWidth &&
              dialog.getBoundingClientRect().left >= 0,
          ),
        `Sharing preview fits at ${width}px`,
      );
      await page.keyboard.press("Escape");
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
    check(
      (await page.locator("#sharing-status").textContent()).includes(
        "2 local drafts",
      ),
      "Reload resets simulated sharing rather than implying durable storage",
    );
    check(
      (await page.locator("#loop-status").textContent()) ===
        "Signal detected — no task yet",
      "Reload resets simulated task completion",
    );
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

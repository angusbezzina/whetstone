// The dashboard reads `wh dash --json` (GET /api/inspect) and writes only
// through `wh change` (POST /api/command) with the session's CSRF token.
// These types mirror src/projection/page.rs; the page invents no state.

export type FieldChange = { field: string; before: string | null; after: string | null };
export type EntryChange = {
  kind: "mission" | "principle" | "rule" | "setting";
  id: string;
  action: "added" | "changed" | "retired" | "reinstated" | "removed";
  title: string;
  fields: FieldChange[];
  reason: string | null;
};
export type HistoryEntry = {
  commit: string;
  short: string;
  author: string;
  at: string;
  subject: string;
  body: string;
  summary: string;
  changes: EntryChange[];
  readable: boolean;
};
export type LastChange = { commit: string; author: string; at: string; subject: string };
export type EditBase = {
  kind: "mission" | "principle" | "rule";
  record_id: string | null;
  content: string;
  rationale: string | null;
  definition: RuleDefinition | Record<string, unknown> | null;
};
export type Enforcer =
  | { kind: "ast"; query: string; language?: string }
  | { kind: "lint"; tool: string; code: string }
  | { kind: "formatter"; tool: string }
  | { kind: "test"; command: string }
  | { kind: "validator"; command: string };
export type SweepUnsure = { paths?: string[]; exclude?: string[]; matching?: string; reason: string };
export type RuleDefinition = {
  type: "rule";
  strength: "must" | "should";
  enforcer: Enforcer;
  examples: unknown[];
  source: unknown;
  paths: string[];
  /** Absent or null: generated code is excluded; [] excludes nothing. */
  exclude?: string[] | null;
  mode?: "tree" | "change";
  batch?: "by-file" | "by-crate" | null;
  sweep: "discuss" | "confirm_first" | "draft_pr" | null;
  sweep_priority: number | null;
  sweep_unsure: SweepUnsure[];
};
export type Example = { input: string; expected: string; reason: string; path: string | null };
export type RuleEvent = {
  commit: string;
  short: string;
  author: string;
  at: string;
  subject: string;
  action: EntryChange["action"];
  fields: FieldChange[];
};
export type Rule = {
  id: string;
  statement: string;
  rationale: string;
  strength: "must" | "should";
  check: { kind: Enforcer["kind"]; label: string; command: string; language: string | null };
  stages: { key: "commit" | "push" | "ci"; label: string; runs: boolean }[];
  paths: string[];
  /** The paths the rule never applies to: its own, or generated code by default. */
  exclude: string[];
  mode: "tree" | "change";
  batch: "by-file" | "by-crate" | null;
  examples: Example[];
  source: { kind: string; label: string; reference: string | null };
  /** The principle the rule serves, when its source names one held in the rulebook. */
  serves: { id: string; statement: string } | null;
  sweep: { class: string; label: string; priority: number; unsure: SweepUnsure[] };
  lifecycle: "accepted" | "changed" | "new";
  digest: string;
  history: RuleEvent[];
  edit: EditBase;
};
export type RuleGroup = { strength: "must" | "should"; label: string; hint: string; rules: Rule[] };
export type HealthItem = { kind: string; tone: "warn" | "fail"; text: string; fix: string | null; detail: string[] };
export type Suggestion = {
  kind: "starter" | "principle";
  id: string;
  title: string;
  detail: string;
  strength: "must" | "should" | null;
  check: string;
  group: string | null;
  starting: boolean;
  edit: EditBase;
};
export type Starter = {
  id: string;
  title: string;
  detected: string;
  strength: "must" | "should";
  enforcer: string;
  accepted: boolean;
  edit: EditBase;
};
export type SetupStep = {
  key: "mission" | "rules" | "gates";
  label: string;
  hint: string;
  state: "done" | "drafted" | "todo";
  command: string | null;
};
export type Principle = {
  id: string;
  statement: string;
  rationale: string | null;
  source: string;
  lifecycle: string;
  edit: EditBase;
  /** The ids of the rules that serve it. */
  served_by: string[];
  enforcer_hint: string | null;
  hint: string;
};
export type Coverage = {
  unserved: { id: string; statement: string }[];
  unanchored: { id: string; statement: string }[];
  require_served: boolean;
};
export type Page = {
  header: {
    project: string;
    branch: string | null;
    revision: string | null;
    summary: string;
    counts: { rules: number; blocks: number; reports: number; principles: number; retired: number };
    last_change: LastChange | null;
  };
  pending: { count: number; summary: string; changes: EntryChange[]; message: { subject: string; body: string } } | null;
  health: { items: HealthItem[]; healthy: string | null };
  mission: { statement: string; lifecycle: string; edit: EditBase } | null;
  principles: Principle[];
  principle_coverage: Coverage;
  groups: RuleGroup[];
  retired: { id: string; reason: string; retired_in: LastChange | null }[];
  suggestions: Suggestion[];
  history: HistoryEntry[];
  setup: { steps: SetupStep[]; starters: Starter[] } | null;
};
export type Envelope<T = unknown> = {
  state: "success" | "needs_input" | "needs_decision" | "unknown" | "failed" | string;
  summary: string;
  blocking_questions: string[];
  permitted_actions: string[];
  data: T;
};
export type DashData = {
  rulebook: { path: string; state: "available" | "missing" | "invalid"; revision: string | null; detail: string | null };
  current: Page | null;
  read_only: boolean;
};

export const RULEBOOK = "whetstone/rulebook.json";

async function json<T>(response: Response): Promise<T> {
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(body.summary || body.reason_code || body.reason || `Request failed (${response.status})`);
  }
  return body as T;
}

export async function inspect(): Promise<Envelope<DashData>> {
  return json(await fetch("/api/inspect"));
}

const TOKEN = "whetstone_csrf";

/** Trade the one-time launch fragment for a session, then ask for edit
 * rights. Resolves to whether this tab may edit. */
export async function startSession(): Promise<{ editable: boolean; reason: string | null }> {
  const fragment = new URLSearchParams(location.hash.slice(1)).get("bootstrap");
  if (fragment) {
    history.replaceState(null, "", location.pathname);
    try {
      const session = await json<{ csrf_token: string }>(
        await fetch("/session/bootstrap", { method: "POST", headers: { "X-Whetstone-Bootstrap": fragment } }),
      );
      sessionStorage.setItem(TOKEN, session.csrf_token);
    } catch (error) {
      return { editable: false, reason: `The launch link was already used (${(error as Error).message}).` };
    }
  }
  const token = sessionStorage.getItem(TOKEN);
  if (!token) return { editable: false, reason: null };
  const response = await fetch("/session/edit", { method: "POST", headers: { "X-Whetstone-CSRF": token } });
  if (response.ok) return { editable: true, reason: null };
  const body = await response.json().catch(() => ({}));
  return {
    editable: false,
    reason: body.reason_code === "read_only" || body.reason === "read_only" ? "This dashboard was started with --read-only." : null,
  };
}

export type ChangeCommand = {
  kind?: "mission" | "principle" | "rule";
  record_id?: string | null;
  content?: string;
  rationale?: string | null;
  definition?: unknown;
  accept?: boolean;
  withdraw?: boolean;
  retire?: string;
  preview?: boolean;
  allow_unvalidated?: boolean;
  from_text?: string;
};

export async function change<T = Record<string, unknown>>(command: ChangeCommand): Promise<Envelope<T>> {
  const token = sessionStorage.getItem(TOKEN);
  if (!token) throw new Error("This tab cannot edit.");
  return json(
    await fetch("/api/command", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Whetstone-CSRF": token },
      body: JSON.stringify({ workflow: "change", ...command }),
    }),
  );
}

// Sharing rules. These mirror src/projection/pack.rs and the review an
// import returns; the page invents no state.
export type PackRule = {
  id: string;
  statement: string;
  strength: "must" | "should";
  enforcer: Enforcer["kind"];
  language: string | null;
  target: string;
  can_leave: boolean;
  reason: string | null;
  serves: { id: string; statement: string } | null;
  files: string[];
  suggested_files: string[];
  tools: string[];
  paths: string[];
  portability: "stays" | "brings_files" | "needs_tools" | "scoped" | "portable";
};
export type PackPlan = {
  revision: string | null;
  name: string;
  version: string;
  rules: PackRule[];
  principles: { id: string; statement: string; origin: string; rules: string[] }[];
};
export type PackChoice = {
  only: string[];
  keep_paths: string[];
  with_files: string[];
  name?: string;
  version?: string;
};
export type ReviewPrinciple = {
  id: string;
  kind: "principle";
  statement: string;
  selected: boolean;
  status: "ready" | "held" | "conflict";
};
export type ReviewRule = {
  id: string;
  kind: "rule";
  statement: string;
  strength: "must" | "should";
  enforcer: Enforcer["kind"];
  target: string;
  runs_command: boolean;
  paths: string[];
  widened: boolean;
  serves: string | null;
  principle_conflict: boolean;
  principle_missing: boolean;
  files: string[];
  selected: boolean;
  status: "ready" | "needs_consent" | "needs_mapping" | "not_validatable" | "file_conflict" | "held";
  reason: string | null;
  suggestion: { from: string; to: string } | null;
  probe: { status?: string; matched?: number; summary?: string };
};
export type ReviewFile = { path: string; bytes: number; executable: boolean; status: "new" | "present" | "conflict"; needed_by: string[] };
export type PackReview = {
  pack: { name: string; version: string; source: string; digest: string };
  entries: (ReviewPrinciple | ReviewRule)[];
  files: ReviewFile[];
};
export type PackImport = {
  review?: PackReview;
  principle_pack?: string;
  drafts?: { id: string; kind: string }[];
  skipped?: { id: string; reason: string }[];
  not_validatable?: { id: string; reason: string | null }[];
  needs_consent?: { id: string; command: string }[];
  file_conflicts?: { id: string; files: string[] }[];
  principle_conflicts?: { rule: string; principle: string; here: string; pack: string }[];
  principle_missing?: { rule: string; principle: string }[];
  fails_today?: { id: string; strength: string; reason: string }[];
  widened?: string[];
  files_written?: string[];
};

export async function packPlan(): Promise<Envelope<PackPlan>> {
  return json(await fetch("/api/pack/plan"));
}

/** Ask for the pack and hand it to the browser as a download. */
export async function downloadPack(choice: PackChoice): Promise<void> {
  const response = await fetch("/api/pack/export", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(choice),
  });
  if (!response.ok) await json(response);
  const name = /filename="([^"]+)"/.exec(response.headers.get("Content-Disposition") ?? "")?.[1] ?? "rules.pack.json";
  const url = URL.createObjectURL(await response.blob());
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  URL.revokeObjectURL(url);
}

export async function importPack(body: {
  pack: string;
  mode: "review" | "apply";
  only?: string[];
  map_paths?: string[];
  allow_commands?: boolean;
}): Promise<Envelope<PackImport>> {
  const token = sessionStorage.getItem(TOKEN);
  if (!token) throw new Error("This tab cannot edit.");
  return json(
    await fetch("/api/pack/import", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Whetstone-CSRF": token },
      body: JSON.stringify(body),
    }),
  );
}

export function slug(text: string): string {
  return (
    text
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .split("-")
      .slice(0, 6)
      .join("-") || "rule"
  );
}

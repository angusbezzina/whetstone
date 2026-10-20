// The dashboard reads `wh dash --json` (GET /api/inspect) and writes only
// through `wh change` (POST /api/command) with the session's CSRF token.
// These types mirror src/projection/page.rs; the page invents no state.

export type FieldChange = { field: string; before: string | null; after: string | null };
export type EntryChange = {
  kind: "mission" | "principle" | "rule";
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
export type SweepUnsure = { paths?: string[]; matching?: string; reason: string };
export type RuleDefinition = {
  type: "rule";
  strength: "must" | "should";
  enforcer: Enforcer;
  examples: unknown[];
  source: unknown;
  paths: string[];
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
  examples: Example[];
  source: { kind: string; label: string; reference: string | null };
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
  principles: { id: string; statement: string; rationale: string | null; source: string; lifecycle: string; edit: EditBase }[];
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
  history.replaceState(null, "", location.pathname);
  if (fragment) {
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

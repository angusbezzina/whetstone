import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChevronDown, CircleAlert } from "lucide-react";
import type { Enforcer, Rule, RuleDefinition } from "@/api";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { gatePass, leader } from "@/motion";
import { day, useDash } from "@/state";
import { InlineText } from "./InlineText";
import { Command, Delta, Field } from "./marks";

const SWEEP_SHORT: Record<string, string> = { discuss: "Discuss", confirm_first: "Ask first", draft_pr: "Fix PR" };
const CHECK_SHORT: Record<string, string> = {
  "a lint": "Lint",
  "a formatter": "Format",
  "a test": "Test",
  "a script": "Script",
  "a command": "Command",
  "a code pattern": "Pattern",
};
const ACTION: Record<string, string> = {
  added: "Added",
  changed: "Changed",
  retired: "Retired",
  reinstated: "Reinstated",
  removed: "Removed",
};
const FIELD: Record<string, string> = {
  statement: "statement",
  rationale: "why",
  strength: "strength",
  enforcer: "check",
  paths: "paths",
  exclude: "excluded paths",
  mode: "mode",
  batch: "batching",
  examples: "examples",
  source: "source",
  sweep: "sweep class",
  sweep_priority: "priority",
  sweep_unsure: "unsure areas",
};

function Chip({ children, strong }: { children: React.ReactNode; strong?: boolean }) {
  return (
    <span className={"display inline-flex h-5 items-center border px-1.5 text-[0.6875rem] leading-none " + (strong ? "border-frame text-foreground" : "border-rule text-ink-2")}>
      {children}
    </span>
  );
}

export function Stages({ rule, pass }: { rule: Rule; pass?: boolean }) {
  const root = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    if (pass && root.current) gatePass(root.current);
  }, [pass]);
  return (
    <span
      ref={root}
      className="relative inline-flex gap-2.5"
      aria-label={"Runs at " + rule.stages.filter((stage) => stage.runs).map((stage) => stage.label).join(", ")}
    >
      {rule.stages.map((stage) => (
        <span key={stage.key} className="stage" data-runs={stage.runs} data-stage>
          {stage.label}
        </span>
      ))}
      {pass ? <span data-gate-line aria-hidden="true" className="pointer-events-none absolute inset-x-0 -bottom-1.5 h-px origin-left scale-x-0 bg-primary" /> : null}
    </span>
  );
}

const SWEEPS = [
  { value: "discuss", label: "Discuss before fixing" },
  { value: "confirm_first", label: "Fix once a person approves" },
  { value: "draft_pr", label: "Fix as a draft PR" },
] as const;

const KINDS: { kind: Enforcer["kind"]; label: string }[] = [
  { kind: "lint", label: "Lint" },
  { kind: "formatter", label: "Formatter" },
  { kind: "test", label: "Test" },
  { kind: "validator", label: "Script" },
  { kind: "ast", label: "Code pattern" },
];

function retarget(kind: Enforcer["kind"], from: Enforcer): Enforcer {
  if (kind === from.kind) return from;
  const text = "command" in from ? from.command : "tool" in from ? from.tool : "query" in from ? from.query : "";
  switch (kind) {
    case "lint":
      return { kind, tool: text, code: "" };
    case "formatter":
      return { kind, tool: text };
    case "test":
    case "validator":
      return { kind, command: text };
    case "ast":
      return { kind, query: text, language: "rust" };
  }
}

/** Writes one field of a rule as a draft; a check that cannot run here is
 * offered again without validation. */
function useRuleEdit(rule: Rule) {
  const { run } = useDash();
  const [refusal, setRefusal] = useState<{ lines: string[]; retry: () => Promise<boolean> } | null>(null);
  const base = rule.edit.definition as RuleDefinition;
  const save = async (
    patch: { content?: string; rationale?: string; definition?: Partial<RuleDefinition> },
    success: string,
    allowUnvalidated = false,
  ): Promise<boolean> => {
    const outcome = await run(
      {
        kind: "rule",
        record_id: rule.id,
        content: patch.content ?? rule.statement,
        rationale: patch.rationale ?? rule.rationale,
        definition: { ...base, ...patch.definition },
        allow_unvalidated: allowUnvalidated || undefined,
      },
      success,
    );
    if (outcome.ok) {
      setRefusal(null);
      return true;
    }
    if (outcome.needsInput) {
      setRefusal({
        lines: outcome.response?.blocking_questions.length ? outcome.response.blocking_questions : [outcome.error ?? "The change was refused."],
        retry: () => save(patch, success, true),
      });
    }
    return false;
  };
  return { save, refusal, dismiss: () => setRefusal(null) };
}

function Retire({ rule, onDone }: { rule: Rule; onDone: () => void }) {
  const { run } = useDash();
  const [why, setWhy] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <form
      className="grid gap-2 border-t border-rule pt-3"
      onSubmit={async (event) => {
        event.preventDefault();
        setBusy(true);
        const outcome = await run({ retire: rule.id, rationale: why.trim() }, "Retired as a draft. Its reason stays in the rulebook.");
        setBusy(false);
        if (outcome.ok) onDone();
      }}
    >
      <label htmlFor={"retire-" + rule.id} className="label">
        Why retire it? The reason stays in the rulebook and the commit.
      </label>
      <Textarea id={"retire-" + rule.id} value={why} onChange={(e) => setWhy(e.target.value)} rows={2} required />
      <div className="flex gap-2">
        <Button type="submit" variant="destructive" disabled={busy || !why.trim()}>
          {busy ? "Retiring…" : "Retire this rule"}
        </Button>
        <Button type="button" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function Detail({ rule, edit, changed }: { rule: Rule; edit: ReturnType<typeof useRuleEdit>; changed: Set<string> }) {
  const { showIds, editable, data } = useDash();
  const principles = data?.current?.principles ?? [];
  const definition = rule.edit.definition as RuleDefinition;
  const enforcer = definition.enforcer;
  const saveEnforcer = (next: Partial<Record<string, string>>) =>
    edit.save({ definition: { enforcer: { ...enforcer, ...next } as Enforcer } }, "The check is drafted.");
  const mono = "font-mono text-[0.8125rem] break-all";
  return (
    <dl className="grid gap-3">
      <Field label="Why">
        <InlineText
          as="p"
          editable={editable}
          label="Why this rule exists"
          value={rule.rationale}
          pending={changed.has("rationale")}
          multiline
          className="max-w-[68ch] text-[1.0625rem] leading-relaxed"
          onSave={(next) => edit.save({ rationale: next }, "The rationale is drafted.")}
        />
      </Field>
      <Field label="Checked by">
        <div className="grid justify-items-start gap-1.5">
          {editable ? (
            <ToggleGroup
              type="single"
              value={enforcer.kind}
              onValueChange={(kind) => kind && kind !== enforcer.kind && edit.save({ definition: { enforcer: retarget(kind as Enforcer["kind"], enforcer) } }, "The check is drafted.")}
              className="flex-wrap justify-start gap-0"
              aria-label="How the rule is checked"
            >
              {KINDS.map((kind) => (
                <ToggleGroupItem
                  key={kind.kind}
                  value={kind.kind}
                  className="h-7 border border-rule px-2 font-display text-[0.6875rem] tracking-[0.06em] uppercase data-[state=on]:border-frame data-[state=on]:bg-foreground data-[state=on]:text-background"
                >
                  {kind.label}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          ) : (
            <span>
              {rule.check.label[0].toUpperCase() + rule.check.label.slice(1)}
              {rule.check.language ? <span className="text-ink-3"> over {rule.check.language}</span> : null}
            </span>
          )}
          {!editable ? (
            <Command text={rule.check.command} />
          ) : enforcer.kind === "lint" ? (
            <span className="flex flex-wrap items-baseline gap-x-2">
              <InlineText editable label="Lint command" value={enforcer.tool} pending={changed.has("enforcer")} className={mono} onSave={(tool) => saveEnforcer({ tool })} />
              <span className="label">code</span>
              <InlineText editable label="Lint code" value={enforcer.code} pending={changed.has("enforcer")} className={mono} onSave={(code) => saveEnforcer({ code })} />
            </span>
          ) : enforcer.kind === "formatter" ? (
            <InlineText editable label="Formatter check command" value={enforcer.tool} pending={changed.has("enforcer")} className={mono} onSave={(tool) => saveEnforcer({ tool })} />
          ) : enforcer.kind === "ast" ? (
            <span className="grid gap-1">
              <InlineText editable label="Tree-sitter query" value={enforcer.query} pending={changed.has("enforcer")} className={mono} onSave={(query) => saveEnforcer({ query })} />
              <span className="flex items-baseline gap-2">
                <span className="label">over</span>
                <InlineText editable label="Language" value={enforcer.language ?? ""} placeholder="language" className={mono} onSave={(language) => saveEnforcer({ language })} />
              </span>
            </span>
          ) : (
            <InlineText editable label="Command" value={enforcer.command} pending={changed.has("enforcer")} className={mono} onSave={(command) => saveEnforcer({ command })} />
          )}
        </div>
      </Field>
      <Field label="Applies to">
        {editable ? (
          <InlineText
            editable
            label="Paths the rule applies to, comma separated"
            value={rule.paths.join(", ")}
            placeholder="The whole repository"
            pending={changed.has("paths")}
            className={mono}
            onSave={(next) =>
              edit.save(
                { definition: { paths: next.split(/[\n,]/).map((path) => path.trim()).filter(Boolean) } },
                "The paths are drafted.",
              )
            }
          />
        ) : rule.paths.length ? (
          <span className="flex flex-wrap gap-1.5">
            {rule.paths.map((path) => (
              <code key={path} className="bare border border-rule bg-card px-1.5 text-[0.8125rem]">
                {path}
              </code>
            ))}
          </span>
        ) : (
          <span className="text-ink-2">The whole repository</span>
        )}
        {rule.exclude.length ? (
          <span className="mt-1 flex flex-wrap items-baseline gap-1.5 text-[0.875rem] text-ink-2">
            except
            {rule.exclude.map((path) => (
              <code key={path} className="bare border border-rule bg-card px-1.5 text-[0.8125rem]">
                {path}
              </code>
            ))}
          </span>
        ) : null}
      </Field>
      <Field label="Runs at">
        <Stages rule={rule} pass />
        <span className="ml-3 text-[0.875rem] text-ink-3">
          {rule.mode === "change" ? "on the change since the base only, " : ""}
          {rule.strength === "must" ? "and fails the gate on a violation" : "and reports, never blocks"}
        </span>
      </Field>
      {rule.strength === "should" ? (
        <Field label="When broken">
          {editable ? (
            <div className="grid justify-items-start gap-2">
              <ToggleGroup
                type="single"
                value={rule.sweep.class}
                onValueChange={(value) => value && value !== rule.sweep.class && edit.save({ definition: { sweep: value === "discuss" ? null : (value as RuleDefinition["sweep"]) } }, "The sweep class is drafted.")}
                className={"flex-wrap justify-start gap-0 " + (changed.has("sweep") ? "redline" : "")}
                aria-label="What an agent may do when the rule is broken"
              >
                {SWEEPS.map((sweep) => (
                  <ToggleGroupItem key={sweep.value} value={sweep.value} className="h-7 border border-rule px-2 font-serif text-[0.875rem] data-[state=on]:border-frame data-[state=on]:bg-foreground data-[state=on]:text-background">
                    {sweep.label}
                  </ToggleGroupItem>
                ))}
              </ToggleGroup>
              <span className="flex items-center gap-2">
                <span className="label">Priority</span>
                <ToggleGroup
                  type="single"
                  value={String(rule.sweep.priority)}
                  onValueChange={(value) => value && Number(value) !== rule.sweep.priority && edit.save({ definition: { sweep_priority: value === "2" ? null : Number(value) } }, "The priority is drafted.")}
                  className={"gap-0 " + (changed.has("sweep_priority") ? "redline" : "")}
                  aria-label="Sweep priority, 0 most urgent"
                >
                  {[0, 1, 2, 3, 4].map((priority) => (
                    <ToggleGroupItem key={priority} value={String(priority)} className="size-7 min-w-7 border border-rule font-display data-[state=on]:border-frame data-[state=on]:bg-foreground data-[state=on]:text-background">
                      {priority}
                    </ToggleGroupItem>
                  ))}
                </ToggleGroup>
                <span className="text-[0.8125rem] text-ink-3">0 is most urgent</span>
              </span>
            </div>
          ) : (
            <span>
              {rule.sweep.label} <span className="text-ink-3">· priority {rule.sweep.priority}</span>
            </span>
          )}
          {rule.mode === "change" ? (
            <span className="mt-1 block text-[0.875rem] text-ink-2">A sweep counts its flags and files no finding.</span>
          ) : rule.batch ? (
            <span className="mt-1 block text-[0.875rem] text-ink-2">One finding per {rule.batch === "by-file" ? "file" : "crate or package"}, listing its sites.</span>
          ) : null}
          {rule.sweep.unsure.length ? (
            <ul className="mt-1 grid gap-0.5 text-[0.875rem] text-ink-2">
              {rule.sweep.unsure.map((unsure, index) => (
                <li key={index}>
                  Unsure in <code>{unsure.paths?.join(", ") || unsure.matching}</code>: {unsure.reason}
                </li>
              ))}
            </ul>
          ) : null}
        </Field>
      ) : null}
      {rule.examples.length ? (
        <Field label="Examples">
          <ul className="grid gap-2">
            {rule.examples.map((example, index) => (
              <li key={index} className="grid gap-1">
                <span className="text-[0.875rem]">
                  <span className={"label mr-2 " + (example.expected === "violates" ? "text-struck" : "")}>{example.expected}</span>
                  {example.reason}
                </span>
                <pre className="overflow-x-auto border border-rule bg-card px-2 py-1.5 text-[0.75rem]">{example.input}</pre>
              </li>
            ))}
          </ul>
        </Field>
      ) : null}
      {editable && principles.length ? (
        <Field label="Serves">
          <select
            aria-label="The principle this rule serves"
            value={rule.serves?.id ?? ""}
            onChange={(e) =>
              e.target.value !== (rule.serves?.id ?? "") &&
              edit.save(
                { definition: { source: e.target.value ? { kind: "principle", reference: e.target.value } : { kind: "owner" } } },
                e.target.value ? "The principle it serves is drafted." : "The rule serves no named principle now.",
              )
            }
            className={"h-8 max-w-full border border-rule bg-background px-2 text-[0.9375rem] text-foreground " + (changed.has("source") ? "redline" : "")}
          >
            <option value="">No principle</option>
            {principles.map((principle) => (
              <option key={principle.id} value={principle.id}>
                {principle.statement.length > 90 ? principle.statement.slice(0, 87) + "…" : principle.statement}
              </option>
            ))}
          </select>
          {rule.serves && showIds ? <code className="ml-2 text-[0.8125rem] text-ink-3">{rule.serves.id}</code> : null}
        </Field>
      ) : rule.serves ? (
        <Field label="Serves">
          <span>
            {rule.serves.statement}
            {showIds ? <code className="ml-2 text-[0.8125rem] text-ink-3">{rule.serves.id}</code> : null}
          </span>
        </Field>
      ) : null}
      <Field label="Source">
        <span>
          {rule.source.label}
          {rule.source.reference && !rule.serves ? <code className="ml-2 text-[0.8125rem] text-ink-3">{rule.source.reference}</code> : null}
        </span>
      </Field>
      <Field label="History">
        {rule.history.length ? (
          <ol className="grid gap-1.5">
            {rule.history.map((event) => (
              <li key={event.commit} className="grid gap-x-3 text-[0.9375rem] sm:grid-cols-[6.5rem_1fr]">
                <span className="text-ink-3">{day(event.at)}</span>
                <span>
                  <span className="label mr-2 text-foreground">{ACTION[event.action]}</span>
                  {event.fields.length ? <span className="text-ink-2">{event.fields.map((field) => FIELD[field.field] ?? field.field).join(", ")} · </span> : null}
                  {event.author} <code className="bare ml-1 text-[0.75rem] text-ink-3">{event.short.slice(0, 7)}</code>
                  <span className="block text-[0.875rem] text-ink-3 italic">{event.subject}</span>
                </span>
              </li>
            ))}
          </ol>
        ) : (
          <span className="text-ink-3 italic">Not committed yet; it is a draft.</span>
        )}
      </Field>
      {showIds ? (
        <Field label="Id · digest">
          <code className="bare text-[0.75rem] break-all text-ink-3">
            {rule.id} · {rule.digest}
          </code>
        </Field>
      ) : null}
    </dl>
  );
}

/** One rule: a single line collapsed; a callout with a leader line open,
 * where every field is edited in place. */
export function RuleRow({ rule, number }: { rule: Rule; number: string }) {
  const { data, editable, showIds, focused } = useDash();
  const [open, setOpen] = useState(false);
  const [retiring, setRetiring] = useState(false);
  const row = useRef<HTMLLIElement>(null);
  const line = useRef<SVGPathElement>(null);
  const callout = useRef<HTMLDivElement>(null);
  // A principle's "served by" link opens this rule and brings it into view.
  useEffect(() => {
    if (focused?.id !== rule.id) return;
    setOpen(true);
    row.current?.scrollIntoView({ block: "center", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    row.current?.querySelector<HTMLElement>("button[aria-controls]")?.focus({ preventScroll: true });
  }, [focused, rule.id]);
  // With principles in the rulebook, a rule that serves none is marked.
  const unanchored =
    (data?.current?.principles.length ?? 0) > 0 &&
    (data?.current?.principle_coverage.unanchored ?? []).some((entry) => entry.id === rule.id);
  const edit = useRuleEdit(rule);
  const panel = "rule-" + rule.id.replace(/[^a-z0-9-]/gi, "-");
  useLayoutEffect(() => {
    if (open) leader(line.current, callout.current);
  }, [open]);
  const drafted = rule.lifecycle !== "accepted";
  const pendingChange = data?.current?.pending?.changes.find((change) => change.id === rule.id);
  const changed = new Set(
    rule.lifecycle === "new" ? ["statement"] : (pendingChange?.fields.map((field) => field.field) ?? []),
  );
  const toggle = () => {
    setOpen(!open);
    setRetiring(false);
  };
  const columns =
    "grid w-full grid-cols-[2.5rem_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1.5 py-3 pr-1 text-left lg:grid-cols-[2.5rem_minmax(0,1fr)_5.5rem_11.5rem_6rem_1.25rem] lg:items-center";
  const marks = (
    <span className="flex h-6 items-center gap-1 lg:h-auto">
      <span className={"display text-[0.8125rem] font-semibold " + (open ? "text-foreground" : "text-ink-3")}>{number}</span>
      {drafted ? <Delta className="size-3" title={rule.lifecycle === "new" ? "New, not committed" : "Edited, not committed"} /> : null}
    </span>
  );
  const chips = (
    <span className="col-start-2 flex flex-wrap items-center gap-x-3 gap-y-1.5 lg:contents">
      <span className="lg:justify-self-start">
        <Chip strong>{CHECK_SHORT[rule.check.label] ?? rule.check.kind}</Chip>
      </span>
      <Stages rule={rule} />
      <span className="lg:justify-self-start">{rule.strength === "should" ? <Chip>{SWEEP_SHORT[rule.sweep.class]}</Chip> : <Chip strong>Fails gate</Chip>}</span>
    </span>
  );
  const chevron = (extra: string) => (
    <ChevronDown aria-hidden="true" className={"size-4 text-ink-3 transition-transform group-hover:text-foreground " + extra + (open ? " rotate-180" : "")} />
  );
  return (
    <li ref={row} data-row className="relative border-b border-rule last:border-b-0">
      {open ? (
        <div className={columns}>
          {marks}
          <span className="min-w-0">
            <InlineText
              editable={editable}
              label="Rule statement"
              value={rule.statement}
              pending={changed.has("statement")}
              className="block text-[1.0625rem] leading-snug font-medium"
              onSave={(next) => edit.save({ content: next }, "The statement is drafted.")}
            />
            {showIds ? <code className="bare mt-0.5 block text-[0.75rem] text-ink-3">{rule.id}</code> : null}
          </span>
          <button type="button" aria-expanded aria-controls={panel} aria-label="Close this rule" onClick={toggle} className="group mt-0.5 cursor-pointer lg:hidden">
            {chevron("")}
          </button>
          {chips}
          <button type="button" aria-expanded aria-controls={panel} aria-label="Close this rule" onClick={toggle} className="group hidden cursor-pointer lg:block">
            {chevron("")}
          </button>
        </div>
      ) : (
        <button type="button" aria-expanded={false} aria-controls={panel} onClick={toggle} className={"group cursor-pointer hover:bg-card/70 " + columns}>
          {marks}
          <span className="min-w-0">
            <span className={"block text-[1.0625rem] leading-snug font-medium " + (changed.has("statement") ? "redline" : "")}>{rule.statement}</span>
            {unanchored ? (
              <span className="mt-0.5 flex items-center gap-1 text-[0.8125rem] text-ink-2">
                <CircleAlert className="size-3.5 shrink-0 text-primary" aria-hidden="true" />
                Serves no principle
              </span>
            ) : null}
            {showIds ? <code className="bare mt-0.5 block text-[0.75rem] text-ink-3">{rule.id}</code> : null}
          </span>
          {chevron("mt-1 lg:hidden")}
          {chips}
          {chevron("hidden lg:block")}
        </button>
      )}
      {open ? (
        <div id={panel} className="relative grid grid-cols-[2.5rem_minmax(0,1fr)] gap-x-3 pb-5">
          <svg className="h-8 w-10 overflow-visible text-frame" aria-hidden="true" viewBox="0 0 40 32">
            <path ref={line} d="M9 -8 V 22 H 40" fill="none" stroke="currentColor" strokeWidth="1" />
          </svg>
          <div ref={callout} className="min-w-0 border border-frame bg-background p-4 sm:p-5">
            {editable ? (
              <p className="label mb-3 normal-case tracking-normal font-serif text-[0.8125rem] text-ink-3">
                Click any value to change it. Enter saves a draft, Esc puts it back.
              </p>
            ) : null}
            <Detail rule={rule} edit={edit} changed={changed} />
            {edit.refusal ? (
              <div role="alert" className="mt-4 border border-struck bg-wash p-3 text-[0.9375rem]">
                <p className="font-medium">Whetstone ran this check once and could not use it here, so nothing was written.</p>
                <ul className="mt-1 list-disc pl-5 text-ink-2">
                  {edit.refusal.lines.map((text) => (
                    <li key={text}>{text}</li>
                  ))}
                </ul>
                <div className="mt-2 flex gap-2">
                  <Button size="sm" variant="outline" onClick={() => edit.refusal?.retry()}>
                    Save it anyway, unvalidated
                  </Button>
                  <Button size="sm" variant="ghost" onClick={edit.dismiss}>
                    Dismiss
                  </Button>
                </div>
              </div>
            ) : null}
            {editable ? (
              retiring ? (
                <div className="mt-4">
                  <Retire rule={rule} onDone={() => setRetiring(false)} />
                </div>
              ) : (
                <div className="mt-4 flex flex-wrap gap-2 border-t border-rule pt-3">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      edit.save(
                        { definition: { strength: rule.strength === "must" ? "should" : "must" } },
                        rule.strength === "must" ? "Drafted: it is now a soft rule." : "Drafted: it is now a hard rule.",
                      )
                    }
                  >
                    {rule.strength === "must" ? "Make it a soft rule" : "Make it a hard rule"}
                  </Button>
                  <Button size="sm" variant="ghost" className="text-struck" onClick={() => setRetiring(true)}>
                    Retire
                  </Button>
                </div>
              )
            ) : null}
          </div>
        </div>
      ) : null}
    </li>
  );
}

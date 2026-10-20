import { useId, useState } from "react";
import { slug, type Enforcer, type RuleDefinition } from "@/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { useDash } from "@/state";

export type RuleDraft = {
  id: string | null;
  statement: string;
  rationale: string;
  definition: RuleDefinition;
};

const CHECKS: { kind: Enforcer["kind"]; label: string }[] = [
  { kind: "lint", label: "Lint" },
  { kind: "formatter", label: "Formatter" },
  { kind: "test", label: "Test" },
  { kind: "validator", label: "Script" },
  { kind: "ast", label: "Code pattern" },
];

const SWEEPS = [
  { value: "discuss", label: "Discuss before fixing", hint: "An agent only posts and asks." },
  { value: "confirm_first", label: "Fix once a person approves", hint: "A person approves the exact plan first." },
  { value: "draft_pr", label: "Fix as a draft PR", hint: "A mechanical fix an agent may open as a draft PR." },
] as const;

export const BLANK: RuleDefinition = {
  type: "rule",
  strength: "should",
  enforcer: { kind: "lint", tool: "", code: "" },
  examples: [],
  source: { kind: "owner" },
  paths: [],
  sweep: null,
  sweep_priority: null,
  sweep_unsure: [],
};

function enforcerFor(kind: Enforcer["kind"], from: Enforcer): Enforcer {
  if (kind === from.kind) return from;
  const text = "command" in from ? from.command : "tool" in from ? from.tool : "";
  switch (kind) {
    case "lint":
      return { kind, tool: text, code: "" };
    case "formatter":
      return { kind, tool: text };
    case "test":
    case "validator":
      return { kind, command: text };
    case "ast":
      return { kind, query: "", language: "rust" };
  }
}

function Row({ label, htmlFor, children, hint }: { label: string; htmlFor?: string; children: React.ReactNode; hint?: string }) {
  return (
    <div className="grid gap-1.5 sm:grid-cols-[8.5rem_1fr] sm:gap-4">
      <label htmlFor={htmlFor} className="label pt-2">
        {label}
      </label>
      <div className="grid min-w-0 gap-1.5">
        {children}
        {hint ? <p className="text-[0.8125rem] text-ink-3">{hint}</p> : null}
      </div>
    </div>
  );
}

/** The rule as a form: every field `wh change` takes, in plain words. Saving
 * writes a draft; nothing is committed until the pending bar accepts it. */
export function RuleForm({
  initial,
  submitLabel,
  onDone,
  onCancel,
  withStrength = true,
}: {
  initial: RuleDraft;
  submitLabel: string;
  onDone: () => void;
  onCancel: () => void;
  withStrength?: boolean;
}) {
  const { run } = useDash();
  const uid = useId();
  const [statement, setStatement] = useState(initial.statement);
  const [rationale, setRationale] = useState(initial.rationale);
  const [definition, setDefinition] = useState<RuleDefinition>(initial.definition);
  const [paths, setPaths] = useState(initial.definition.paths.join("\n"));
  const [busy, setBusy] = useState(false);
  const [refusal, setRefusal] = useState<string[] | null>(null);
  const enforcer = definition.enforcer;
  const setEnforcer = (next: Partial<Record<string, string>>) =>
    setDefinition({ ...definition, enforcer: { ...enforcer, ...next } as Enforcer });

  const save = async (allowUnvalidated: boolean) => {
    setBusy(true);
    const body: RuleDefinition = {
      ...definition,
      paths: paths
        .split(/[\n,]/)
        .map((path) => path.trim())
        .filter(Boolean),
    };
    const outcome = await run(
      {
        kind: "rule",
        record_id: initial.id ?? `rule.${slug(statement)}`,
        content: statement.trim(),
        rationale: rationale.trim() || null,
        definition: body,
        allow_unvalidated: allowUnvalidated || undefined,
      },
      initial.id ? "Saved as a draft. Review it in the pending changes." : "Added as a draft. Review it in the pending changes.",
    );
    setBusy(false);
    if (outcome.ok) onDone();
    else if (outcome.needsInput)
      setRefusal(outcome.response?.blocking_questions.length ? outcome.response.blocking_questions : [outcome.error ?? "The change was refused."]);
  };

  return (
    <form
      className="grid gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        save(false);
      }}
    >
      <Row label="Rule" htmlFor={uid + "s"}>
        <Textarea id={uid + "s"} value={statement} onChange={(e) => setStatement(e.target.value)} rows={2} required className="text-[1.0625rem]" />
      </Row>
      <Row label="Why" htmlFor={uid + "r"} hint="Agents and reviewers see this next to every violation.">
        <Textarea id={uid + "r"} value={rationale} onChange={(e) => setRationale(e.target.value)} rows={2} required />
      </Row>
      {withStrength ? (
        <Row label="Strength">
          <RadioGroup
            value={definition.strength}
            onValueChange={(value) => setDefinition({ ...definition, strength: value as "must" | "should" })}
            className="grid gap-2 sm:grid-cols-2"
          >
            {(
              [
                ["must", "Hard rule", "Must hold. A violation fails the commit, the push or CI."],
                ["should", "Soft rule", "Should hold. A violation is reported and never blocks."],
              ] as const
            ).map(([value, label, hint]) => (
              <label key={value} className="flex cursor-pointer items-start gap-2 border border-rule p-2.5 has-[[data-state=checked]]:border-frame">
                <RadioGroupItem value={value} className="mt-1" />
                <span>
                  <span className="display block text-[0.8125rem] font-medium">{label} ({value})</span>
                  <span className="text-[0.875rem] text-ink-2">{hint}</span>
                </span>
              </label>
            ))}
          </RadioGroup>
        </Row>
      ) : null}
      <Row label="Checked by">
        <ToggleGroup
          type="single"
          value={enforcer.kind}
          onValueChange={(kind) => kind && setDefinition({ ...definition, enforcer: enforcerFor(kind as Enforcer["kind"], enforcer) })}
          className="flex-wrap justify-start gap-0"
          aria-label="How the rule is checked"
        >
          {CHECKS.map((check) => (
            <ToggleGroupItem
              key={check.kind}
              value={check.kind}
              className="border border-rule font-display text-[0.75rem] tracking-[0.06em] uppercase data-[state=on]:border-frame data-[state=on]:bg-foreground data-[state=on]:text-background"
            >
              {check.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        {enforcer.kind === "lint" ? (
          <div className="grid gap-2 sm:grid-cols-[1fr_12rem]">
            <Input aria-label="Lint command" placeholder="cargo clippy --all-targets -- -D warnings" value={enforcer.tool} onChange={(e) => setEnforcer({ tool: e.target.value })} className="font-mono text-[0.8125rem]" required />
            <Input aria-label="Lint code" placeholder="dead_code" value={enforcer.code} onChange={(e) => setEnforcer({ code: e.target.value })} className="font-mono text-[0.8125rem]" required />
          </div>
        ) : enforcer.kind === "formatter" ? (
          <Input aria-label="Formatter check command" placeholder="cargo fmt --all --check" value={enforcer.tool} onChange={(e) => setEnforcer({ tool: e.target.value })} className="font-mono text-[0.8125rem]" required />
        ) : enforcer.kind === "ast" ? (
          <div className="grid gap-2">
            <Textarea aria-label="Tree-sitter query" placeholder="((macro_invocation macro: (identifier) @name) @match (#eq? @name &quot;dbg&quot;))" value={enforcer.query} onChange={(e) => setEnforcer({ query: e.target.value })} className="font-mono text-[0.8125rem]" rows={3} required />
            <Input aria-label="Language" placeholder="rust" value={enforcer.language ?? ""} onChange={(e) => setEnforcer({ language: e.target.value })} className="w-40 font-mono text-[0.8125rem]" />
          </div>
        ) : (
          <Input aria-label="Command" placeholder={enforcer.kind === "test" ? "cargo test" : "sh whetstone/checks/my-check.sh"} value={enforcer.command} onChange={(e) => setEnforcer({ command: e.target.value })} className="font-mono text-[0.8125rem]" required />
        )}
      </Row>
      <Row label="Applies to" htmlFor={uid + "p"} hint="One glob per line. Leave empty for the whole repository.">
        <Textarea id={uid + "p"} value={paths} onChange={(e) => setPaths(e.target.value)} rows={2} placeholder="(empty: the whole repository)" className="font-mono text-[0.8125rem]" />
      </Row>
      {definition.strength === "should" ? (
        <Row label="When broken">
          <RadioGroup
            value={definition.sweep ?? "discuss"}
            onValueChange={(value) => setDefinition({ ...definition, sweep: value === "discuss" ? null : (value as RuleDefinition["sweep"]) })}
            className="grid gap-1.5"
          >
            {SWEEPS.map((sweep) => (
              <label key={sweep.value} className="flex cursor-pointer items-baseline gap-2">
                <RadioGroupItem value={sweep.value} className="translate-y-0.5" />
                <span>
                  {sweep.label} <span className="text-[0.875rem] text-ink-3">{sweep.hint}</span>
                </span>
              </label>
            ))}
          </RadioGroup>
          <div className="flex items-center gap-3">
            <span className="label">Priority</span>
            <ToggleGroup
              type="single"
              value={String(definition.sweep_priority ?? 2)}
              onValueChange={(value) => value && setDefinition({ ...definition, sweep_priority: value === "2" ? null : Number(value) })}
              className="gap-0"
              aria-label="Sweep priority, 0 most urgent"
            >
              {[0, 1, 2, 3, 4].map((priority) => (
                <ToggleGroupItem key={priority} value={String(priority)} className="size-8 min-w-8 border border-rule font-display data-[state=on]:border-frame data-[state=on]:bg-foreground data-[state=on]:text-background">
                  {priority}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
            <span className="text-[0.8125rem] text-ink-3">0 is most urgent</span>
          </div>
        </Row>
      ) : null}
      {refusal ? (
        <div role="alert" className="border border-struck bg-wash p-3 text-[0.9375rem]">
          <p className="font-medium">Whetstone ran this check once and could not use it here, so nothing was written.</p>
          <ul className="mt-1 list-disc pl-5 text-ink-2">
            {refusal.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          <Button type="button" variant="link" className="mt-1" disabled={busy} onClick={() => save(true)}>
            Save it anyway, unvalidated
          </Button>
        </div>
      ) : null}
      <div className="flex flex-wrap gap-2 sm:pl-[10.5rem]">
        <Button type="submit" disabled={busy || !statement.trim()}>
          {busy ? "Checking…" : submitLabel}
        </Button>
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

import { useState } from "react";
import type { Suggestion } from "@/api";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { ChevronDown } from "lucide-react";
import { useDash } from "@/state";
import { BLANK, RuleForm } from "./RuleForm";

function Adopt({ suggestion }: { suggestion: Suggestion }) {
  const { run } = useDash();
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState<string | null>(null);
  const adopt = async (allowUnvalidated = false) => {
    setBusy(true);
    const edit = suggestion.edit;
    const outcome = await run(
      {
        kind: edit.kind,
        record_id: edit.record_id ?? suggestion.id,
        content: edit.content,
        rationale: edit.rationale,
        definition: edit.definition,
        allow_unvalidated: allowUnvalidated || undefined,
      },
      suggestion.kind === "starter" ? "Adopted as a draft rule." : "Adopted as a draft principle.",
    );
    setBusy(false);
    if (outcome.needsInput) setRefused(outcome.response?.blocking_questions.join(" ") || outcome.error);
  };
  return (
    <div className="grid gap-1 justify-items-end">
      <Button size="sm" variant={suggestion.kind === "starter" ? "default" : "outline"} disabled={busy} onClick={() => adopt()}>
        {busy ? "Checking…" : "Adopt"}
      </Button>
      {refused ? (
        <p role="alert" className="max-w-64 text-right text-[0.8125rem] text-struck">
          {refused}{" "}
          <button type="button" className="cursor-pointer underline" onClick={() => adopt(true)}>
            Adopt unvalidated
          </button>
        </p>
      ) : null}
    </div>
  );
}

/** The principles the loaded packs offer as a starting point, adopted in
 * one go as drafts. Offered until the rulebook holds a principle. */
export function StartingSet() {
  const { data, editable, run, announce } = useDash();
  const [busy, setBusy] = useState(false);
  const page = data?.current;
  const starting = (page?.suggestions ?? []).filter((suggestion) => suggestion.kind === "principle" && suggestion.starting);
  if (!editable || !page || page.principles.length || !starting.length) return null;
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          let adopted = 0;
          for (const suggestion of starting) {
            const edit = suggestion.edit;
            const outcome = await run(
              { kind: "principle", record_id: suggestion.id, content: edit.content, rationale: edit.rationale, definition: edit.definition },
              `Adopted ${suggestion.title.slice(0, 40)}…`,
            );
            if (outcome.ok) adopted += 1;
            else break;
          }
          setBusy(false);
          announce(`Adopted ${adopted} of ${starting.length} starting principles as drafts.`);
        }}
      >
        {busy ? "Adopting…" : `Adopt the starting set (${starting.length})`}
      </Button>
      <span className="text-[0.875rem] text-ink-3">
        {starting.length} principles the packs suggest to begin with; edit or retire any of them afterwards.
      </span>
    </div>
  );
}

export function SuggestionRow({ suggestion }: { suggestion: Suggestion }) {
  return (
    <li className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-4 border-b border-rule py-3 last:border-b-0">
      <div className="min-w-0">
        <p className="font-medium">{suggestion.title}</p>
        <p className="mt-0.5 text-[0.875rem] text-ink-2">
          {suggestion.strength ? (
            <span className="label mr-2 text-foreground">{suggestion.strength === "must" ? "Hard rule (must)" : "Soft rule (should)"}</span>
          ) : null}
          {suggestion.starting ? <span className="label mr-2 text-foreground">Starting set</span> : null}
          {suggestion.check}
        </p>
        {suggestion.kind === "starter" ? <p className="mt-0.5 text-[0.8125rem] text-ink-3 italic">{suggestion.detail}</p> : null}
      </div>
      <Adopt suggestion={suggestion} />
    </li>
  );
}

function Guidelines() {
  const { run } = useDash();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ summary: string; needs: string[] } | null>(null);
  return (
    <form
      className="grid gap-3"
      onSubmit={async (event) => {
        event.preventDefault();
        setBusy(true);
        const outcome = await run({ from_text: text }, "Compiled the guidelines into draft rules.");
        setBusy(false);
        const data = (outcome.response?.data ?? {}) as { needs_enforcer?: { prose: string }[]; not_validatable?: { prose: string; probe: { reason: string } }[] };
        setResult({
          summary: outcome.response?.summary ?? outcome.error ?? "",
          needs: [
            ...(data.needs_enforcer ?? []).map((item) => `Needs a check: ${item.prose}`),
            ...(data.not_validatable ?? []).map((item) => `Could not be checked here: ${item.prose} (${item.probe.reason})`),
          ],
        });
      }}
    >
      <label htmlFor="guidelines" className="text-[0.9375rem] text-ink-2">
        Paste guidance from <code>AGENTS.md</code>, a style guide or a review comment. Each line that names a tool becomes a draft rule; the rest is listed for you to give a check.
      </label>
      <Textarea id="guidelines" value={text} onChange={(e) => setText(e.target.value)} rows={5} className="font-mono text-[0.8125rem]" />
      <div>
        <Button type="submit" disabled={busy || !text.trim()}>
          {busy ? "Compiling…" : "Compile into drafts"}
        </Button>
      </div>
      {result ? (
        <div className="border-t border-rule pt-3 text-[0.9375rem]" role="status">
          <p>{result.summary}</p>
          {result.needs.length ? (
            <ul className="mt-1 list-disc pl-5 text-ink-2">
              {result.needs.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
    </form>
  );
}

/** Add rule: suggestions detected for this repository, principles from the
 * loaded packs, a blank rule, or rules compiled from prose. */
export function AddRule({ onClose }: { onClose: () => void }) {
  const { data } = useDash();
  const suggestions = data?.current?.suggestions ?? [];
  const starters = suggestions.filter((suggestion) => suggestion.kind === "starter");
  const principles = suggestions.filter((suggestion) => suggestion.kind === "principle");
  const pack = principles[0]?.detail;
  return (
    <section aria-labelledby="add-rule-title" className="border border-frame bg-background p-4 sm:p-5">
      <div className="mb-3 flex items-baseline justify-between gap-4">
        <h3 id="add-rule-title" className="display text-[1.125rem] font-medium">Add a rule</h3>
        <Button variant="ghost" size="sm" onClick={onClose}>
          Close
        </Button>
      </div>
      <Tabs defaultValue={starters.length ? "suggested" : "write"}>
        <TabsList variant="line" className="mb-3 h-auto w-full justify-start gap-4 border-b border-rule p-0">
          {[
            ["suggested", `Suggested (${suggestions.length})`],
            ["write", "Write one"],
            ["guidelines", "From your guidelines"],
          ].map(([value, label]) => (
            <TabsTrigger
              key={value}
              value={value}
              className="display -mb-px flex-none rounded-none border-0 border-b-2 border-transparent px-0 pb-2 text-[0.8125rem] data-[state=active]:border-primary data-[state=active]:text-foreground"
            >
              {label}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value="suggested">
          {starters.length ? (
            <>
              <p className="label mb-1">Detected in this repository</p>
              <ul>
                {starters.map((suggestion) => (
                  <SuggestionRow key={suggestion.id} suggestion={suggestion} />
                ))}
              </ul>
            </>
          ) : (
            <p className="text-ink-2 italic">Every starter rule detected here is already in the rulebook.</p>
          )}
          {principles.length ? (
            <Collapsible className="mt-4 border-t border-rule pt-3">
              <CollapsibleTrigger className="group flex w-full cursor-pointer items-baseline justify-between gap-4 text-left">
                <span>
                  <span className="label mr-2 text-foreground">Principles</span>
                  <span className="text-[0.9375rem] text-ink-2">
                    {principles.length} from {pack}. Principles are not enforced; a rule can serve one.
                  </span>
                </span>
                <ChevronDown className="size-4 shrink-0 text-ink-3 transition-transform group-data-[state=open]:rotate-180" />
              </CollapsibleTrigger>
              <CollapsibleContent>
                <ul className="mt-2">
                  {principles.map((suggestion) => (
                    <SuggestionRow key={suggestion.id} suggestion={suggestion} />
                  ))}
                </ul>
              </CollapsibleContent>
            </Collapsible>
          ) : null}
        </TabsContent>
        <TabsContent value="write">
          <RuleForm initial={{ id: null, statement: "", rationale: "", definition: BLANK }} submitLabel="Add as a draft" onDone={onClose} onCancel={onClose} />
        </TabsContent>
        <TabsContent value="guidelines">
          <Guidelines />
        </TabsContent>
      </Tabs>
    </section>
  );
}

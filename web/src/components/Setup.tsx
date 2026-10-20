import { useState } from "react";
import { Check } from "lucide-react";
import type { SetupStep, Starter } from "@/api";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { useDash } from "@/state";
import { AddRule, StartingSet } from "./AddRule";
import { Command, Delta } from "./marks";

function Step({ step, index, children }: { step: SetupStep; index: number; children: React.ReactNode }) {
  return (
    <li className="grid gap-3 border-b border-rule py-6 last:border-b-0 lg:grid-cols-[10rem_minmax(0,1fr)] lg:gap-8">
      <div className="flex items-baseline gap-3 lg:block">
        <span className="display text-[2rem] leading-none font-semibold text-ink-3">{index}</span>
        <div className="lg:mt-2">
          <h3 className="display text-[1.125rem] font-medium">{step.label}</h3>
          <p className="label mt-1 flex items-center gap-1.5">
            {step.state === "done" ? <Check className="size-3 text-foreground" /> : step.state === "drafted" ? <Delta className="size-3" /> : null}
            {step.state === "done" ? "Committed" : step.state === "drafted" ? "Drafted" : "To do"}
          </p>
        </div>
      </div>
      <div className="min-w-0">
        <p className="mb-3 text-ink-2">{step.hint}</p>
        {children}
      </div>
    </li>
  );
}

function Mission() {
  const { data, editable, run } = useDash();
  const current = data?.current?.mission?.statement ?? "";
  const [text, setText] = useState(current);
  const [busy, setBusy] = useState(false);
  if (!editable)
    return current ? <p className="text-[1.375rem] leading-snug">{current}</p> : <Command text={'wh change --kind mission --content "…"'} />;
  return (
    <form
      className="grid gap-2"
      onSubmit={async (event) => {
        event.preventDefault();
        setBusy(true);
        await run({ kind: "mission", content: text.trim() }, "The mission is drafted.");
        setBusy(false);
      }}
    >
      <label htmlFor="setup-mission" className="sr-only">
        Mission
      </label>
      <Textarea
        id="setup-mission"
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={2}
        placeholder="What this project exists to do, in one sentence."
        className="text-[1.25rem] leading-snug"
      />
      <div>
        <Button type="submit" variant={current ? "outline" : "default"} disabled={busy || !text.trim() || text.trim() === current}>
          {busy ? "Saving…" : current ? "Update the mission" : "Draft the mission"}
        </Button>
      </div>
    </form>
  );
}

function StarterRow({ starter }: { starter: Starter }) {
  const { editable, run } = useDash();
  const [busy, setBusy] = useState(false);
  return (
    <li className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-4 border-b border-rule py-3 last:border-b-0">
      <div className="min-w-0">
        <p className="font-medium">{starter.title}</p>
        <p className="mt-0.5 text-[0.875rem] text-ink-2">
          <span className="label mr-2 text-foreground">{starter.strength === "must" ? "Hard rule (must)" : "Soft rule (should)"}</span>
          Checked by {starter.enforcer.toLowerCase()} · {starter.detected}
        </p>
      </div>
      {starter.accepted ? (
        <span className="label flex items-center gap-1 pt-1.5">
          <Delta className="size-3" /> In the draft
        </span>
      ) : editable ? (
        <Button
          size="sm"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            const edit = starter.edit;
            await run({ kind: "rule", record_id: edit.record_id, content: edit.content, rationale: edit.rationale, definition: edit.definition }, "Adopted as a draft rule.");
            setBusy(false);
          }}
        >
          {busy ? "Checking…" : "Adopt"}
        </Button>
      ) : null}
    </li>
  );
}

/** The first-run page: mission, rules, gates, then one commit. It is shown
 * only until a mission and a rule are committed. */
export function Setup() {
  const { data, editable, run } = useDash();
  const page = data?.current;
  const [adding, setAdding] = useState(false);
  const [busy, setBusy] = useState(false);
  if (!page?.setup) return null;
  const [mission, rules, gates] = page.setup.steps;
  const ready = mission.state !== "todo" && rules.state !== "todo";
  return (
    <section aria-labelledby="setup-title" className="px-4 py-8 sm:px-8 lg:px-10">
      <div className="grid gap-2 lg:grid-cols-[10rem_minmax(0,1fr)] lg:gap-8">
        <span className="label pt-3">Setup</span>
        <div>
          <h2 id="setup-title" className="text-[2rem] leading-tight font-medium text-balance sm:text-[2.5rem]">
            This repository has no rulebook yet.
          </h2>
          <p className="mt-3 max-w-[64ch] text-[1.0625rem] text-ink-2">
            Write the mission, adopt the rules that hold here, and commit them. They live in <code>whetstone/rulebook.json</code> and are checked at commit, push and CI. Nothing is committed until you say so.
          </p>
        </div>
      </div>
      <ol className="mt-6">
        <Step step={mission} index={1}>
          <Mission />
          <div className="mt-4">
            <StartingSet />
          </div>
        </Step>
        <Step step={rules} index={2}>
          <ul>
            {page.setup.starters.map((starter) => (
              <StarterRow key={starter.id} starter={starter} />
            ))}
          </ul>
          {editable ? (
            adding ? (
              <div className="mt-4">
                <AddRule onClose={() => setAdding(false)} />
              </div>
            ) : (
              <Button variant="outline" size="sm" className="mt-3" onClick={() => setAdding(true)}>
                Write your own rule
              </Button>
            )
          ) : null}
        </Step>
        <Step step={gates} index={3}>
          {gates.command ? <Command text={gates.command} /> : null}
          <p className="mt-2 text-[0.875rem] text-ink-3">Run it in a terminal. The hooks run the rules on every commit and push.</p>
        </Step>
      </ol>
      {editable ? (
        <div className="mt-2 flex flex-wrap items-center gap-4 border-t border-frame pt-5 lg:pl-[12rem]">
          <Button
            disabled={!ready || busy || !page.pending}
            onClick={async () => {
              setBusy(true);
              await run({ accept: true, rationale: null }, "The rulebook is committed.");
              setBusy(false);
            }}
          >
            {busy ? "Committing…" : "Commit the rulebook"}
          </Button>
          <span className="text-[0.9375rem] text-ink-2">
            {ready ? "Commits whetstone/rulebook.json and nothing else." : "Draft a mission and at least one rule first."}
          </span>
        </div>
      ) : null}
    </section>
  );
}

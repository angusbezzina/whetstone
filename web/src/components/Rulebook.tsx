import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { CircleAlert, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cascade } from "@/motion";
import type { Principle } from "@/api";
import { day, useDash } from "@/state";
import { AddRule, StartingSet, SuggestionRow } from "./AddRule";
import { InlineText } from "./InlineText";
import { Delta } from "./marks";
import { RuleRow } from "./RuleRow";
import { Section } from "./Section";

const heading = "display text-[1.375rem] leading-none font-semibold";

function Mission() {
  const { data, editable, run } = useDash();
  const mission = data?.current?.mission;
  if (!mission) return null;
  return (
    <Section id="mission" collapsible={false} title={<span className="label sr-only pt-4 lg:not-sr-only lg:block">Mission</span>}>
      <InlineText
        as="p"
        editable={editable}
        label="Mission"
        value={mission.statement}
        pending={mission.lifecycle !== "accepted"}
        className="max-w-[44ch] text-[1.5rem] leading-[1.25] font-medium text-balance sm:text-[1.875rem] lg:text-[2.125rem]"
        onSave={async (next) => (await run({ kind: "mission", content: next }, "The mission is drafted.")).ok}
      />
      <p className="mt-3 text-[0.9375rem] text-ink-2 italic">
        Every rule below exists to serve this.
        {editable ? <span className="not-italic text-ink-3"> Click the mission to change it.</span> : null}
      </p>
    </Section>
  );
}

/** The rules that serve a principle, each a link that opens its row; or,
 * when none does, the mark and the way to write one. */
function ServedBy({ principle }: { principle: Principle }) {
  const { data, editable, focusRule, compose } = useDash();
  const rules = (data?.current?.groups ?? []).flatMap((group) =>
    group.rules.map((rule, position) => ({ rule, number: (group.strength === "must" ? "H" : "S") + (position + 1) })),
  );
  if (principle.served_by.length) {
    return (
      <span className="mt-0.5 flex flex-wrap items-baseline gap-x-2 gap-y-0.5 text-[0.8125rem] text-ink-2">
        <span className="label">Served by</span>
        {principle.served_by.map((id) => {
          const found = rules.find((entry) => entry.rule.id === id);
          return (
            <button key={id} type="button" onClick={() => focusRule(id)} className="cursor-pointer text-left underline decoration-rule underline-offset-4 hover:decoration-foreground">
              {found ? `${found.number} ${found.rule.statement}` : id}
            </button>
          );
        })}
      </span>
    );
  }
  return (
    <span className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[0.8125rem] text-ink-2">
      <CircleAlert className="size-3.5 shrink-0 text-primary" aria-label="No rule serves this" />
      <span>No rule serves this yet. {principle.hint}.</span>
      {editable ? (
        <Button variant="outline" size="sm" className="h-7" onClick={() => compose({ serves: principle.id, enforcer: principle.enforcer_hint })}>
          <Plus /> Write a rule for it
        </Button>
      ) : null}
    </span>
  );
}

function Principles() {
  const { data, editable, showIds, run } = useDash();
  const principles = data?.current?.principles ?? [];
  const offered = (data?.current?.suggestions ?? []).filter((suggestion) => suggestion.kind === "principle");
  const [all, setAll] = useState(false);
  const [adopting, setAdopting] = useState(false);
  const shown = all ? principles : principles.slice(0, 5);
  const pending = new Set((data?.current?.pending?.changes ?? []).filter((change) => change.kind === "principle").map((change) => change.id));
  return (
    <Section
      id="principles"
      title={<span className={heading}>Principles</span>}
      meta={<p className="text-[0.875rem] text-ink-2">What the rules serve. No tool checks a principle; the rules that serve it do.</p>}
      folded={`${principles.length} ${principles.length === 1 ? "principle" : "principles"}`}
    >
      {principles.length ? (
        <ul className="grid max-w-[72ch] gap-2">
          {shown.map((principle) => (
            <li key={principle.id} className="grid grid-cols-[1rem_minmax(0,1fr)] gap-2">
              {principle.lifecycle !== "accepted" ? <Delta className="mt-1.5" /> : <span className="mt-2.5 size-1.5 bg-foreground" aria-hidden="true" />}
              <span>
                <InlineText
                  editable={editable}
                  label="Principle"
                  value={principle.statement}
                  pending={pending.has(principle.id)}
                  onSave={async (next) =>
                    (
                      await run(
                        {
                          kind: "principle",
                          record_id: principle.id,
                          content: next,
                          rationale: principle.edit.rationale,
                          definition: principle.edit.definition,
                        },
                        "The principle is drafted.",
                      )
                    ).ok
                  }
                />
                <span className="ml-2 text-[0.8125rem] text-ink-3">
                  {principle.source}
                  {showIds ? ` · ${principle.id}` : ""}
                </span>
                <ServedBy principle={principle} />
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <div className="grid justify-items-start gap-3">
          <p className="text-ink-2 italic">No principles yet. Adopt the starting set or one from a pack, or the rules stand on the mission alone.</p>
          <StartingSet />
        </div>
      )}
      <div className="mt-2 flex flex-wrap gap-2">
        {principles.length > shown.length ? (
          <Button variant="ghost" size="sm" className="-ml-3" onClick={() => setAll(true)}>
            Show all {principles.length}
          </Button>
        ) : null}
        {editable && offered.length ? (
          <Button variant="ghost" size="sm" className="-ml-3" aria-expanded={adopting} onClick={() => setAdopting(!adopting)}>
            <Plus /> {adopting ? "Done adopting" : "Adopt a principle"}
          </Button>
        ) : null}
      </div>
      {adopting ? (
        <ul className="mt-2 border-t border-rule">
          {offered.map((suggestion) => (
            <SuggestionRow key={suggestion.id} suggestion={suggestion} />
          ))}
        </ul>
      ) : null}
    </Section>
  );
}

function Groups() {
  const { data, editable, composing, compose, focused } = useDash();
  const groups = data?.current?.groups ?? [];
  const [adding, setAdding] = useState(false);
  const list = useRef<HTMLDivElement>(null);
  const form = useRef<HTMLDivElement>(null);
  // A principle's "Write a rule for it" opens Add rule, pre-filled.
  useEffect(() => {
    if (composing) setAdding(true);
  }, [composing]);
  // Once the form is on the page, bring it into view.
  useEffect(() => {
    if (composing && adding) {
      form.current?.scrollIntoView({ block: "start", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    }
  }, [composing, adding]);
  const close = () => {
    setAdding(false);
    compose(null);
  };
  const played = useRef(false);
  useLayoutEffect(() => {
    if (!played.current && list.current && groups.some((group) => group.rules.length)) {
      played.current = true;
      cascade(list.current.querySelectorAll("[data-row]"));
    }
  }, [groups]);
  return (
    <div ref={list} className="grid gap-10">
      {groups.map((group, index) => (
        <Section
          key={group.strength}
          id={"group-" + group.strength}
          reveal={
            focused && group.rules.some((rule) => rule.id === focused.id)
              ? focused.key
              : index === 0 && composing
                ? composing.key
                : null
          }
          title={
            <span className={heading}>
              {group.label} <span className="text-[1rem] text-ink-3">({group.strength})</span>
            </span>
          }
          meta={<p className="text-[0.875rem] text-ink-2">{group.hint}</p>}
          folded={`${group.rules.length} ${group.label.toLowerCase().slice(0, group.rules.length === 1 ? -1 : undefined)}`}
        >
          {index === 0 && editable ? (
            <div className="mb-3 flex justify-end">
              <Button size="sm" variant={adding ? "outline" : "default"} aria-expanded={adding} onClick={() => (adding ? close() : setAdding(true))}>
                <Plus /> Add rule
              </Button>
            </div>
          ) : null}
          {index === 0 && adding ? (
            <div ref={form} className="mb-4 scroll-mt-4">
              <AddRule key={composing?.key ?? "blank"} onClose={close} />
            </div>
          ) : null}
          <div className="label mb-1 hidden grid-cols-[2.5rem_minmax(0,1fr)_5.5rem_11.5rem_6rem_1.25rem] gap-x-3 border-b border-frame pr-1 pb-1.5 lg:grid">
            <span>No.</span>
            <span>Rule</span>
            <span>Checked by</span>
            <span>Runs at</span>
            <span>When broken</span>
          </div>
          <div className="border-b border-frame lg:hidden" />
          {group.rules.length ? (
            <ul>
              {group.rules.map((rule, position) => (
                <RuleRow key={rule.id} rule={rule} number={(group.strength === "must" ? "H" : "S") + (position + 1)} />
              ))}
            </ul>
          ) : (
            <p className="py-3 text-ink-2 italic">
              {group.strength === "must" ? "No hard rule yet. Nothing here stops a commit or a push." : "No soft rule yet."}
            </p>
          )}
        </Section>
      ))}
    </div>
  );
}

function Retired() {
  const { data } = useDash();
  const retired = data?.current?.retired ?? [];
  if (!retired.length) return null;
  const count = `${retired.length} ${retired.length === 1 ? "rule" : "rules"} taken out, with the reason`;
  return (
    <Section
      id="retired"
      title={<span className={heading}>Retired</span>}
      folded={count}
    >
      <ul className="border-t border-frame">
        {retired.map((entry) => (
          <li key={entry.id} className="grid gap-1 border-b border-rule py-2.5 sm:grid-cols-[minmax(0,16rem)_minmax(0,1fr)] sm:gap-4">
            <code className="del bare text-[0.8125rem]">{entry.id}</code>
            <span className="text-[0.9375rem]">
              {entry.reason}
              {entry.retired_in ? (
                <span className="ml-2 text-[0.8125rem] text-ink-3">
                  {day(entry.retired_in.at)} · {entry.retired_in.author}
                </span>
              ) : null}
            </span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

export function Rulebook() {
  return (
    <div className="grid gap-12">
      <Mission />
      <Principles />
      <Groups />
      <Retired />
    </div>
  );
}

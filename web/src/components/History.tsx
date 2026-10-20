import { useLayoutEffect, useRef, useState } from "react";
import { ChevronDown, Download } from "lucide-react";
import type { HistoryEntry } from "@/api";
import { Button } from "@/components/ui/button";
import { stamp } from "@/motion";
import { clock, day, useDash } from "@/state";
import { DiffList } from "./Diff";
import { Section } from "./Section";
import { Delta, Prose } from "./marks";

function Revision({ entry, fresh, number }: { entry: HistoryEntry; fresh: boolean; number: number }) {
  const [open, setOpen] = useState(false);
  const mark = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    if (fresh) stamp(mark.current);
  }, [fresh]);
  const panel = "rev-" + entry.short;
  return (
    <li className="border-b border-rule last:border-b-0">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panel}
        onClick={() => setOpen(!open)}
        className="group grid w-full cursor-pointer grid-cols-[6rem_minmax(0,1fr)_1rem] gap-x-3 gap-y-0.5 py-2.5 text-left hover:bg-card/70 sm:grid-cols-[6rem_7.5rem_9rem_minmax(0,1fr)_1rem]"
      >
        <span className="flex items-center gap-1.5">
          <span ref={mark} className="inline-flex">
            <Delta number={number} title={`Revision ${number}`} className={"size-5 " + (fresh ? "" : "text-ink-2")} />
          </span>
          <code className="bare text-[0.8125rem]">{entry.short.slice(0, 7)}</code>
        </span>
        <span className="hidden text-[0.9375rem] sm:block">
          {day(entry.at)} <span className="text-ink-3">{clock(entry.at)}</span>
        </span>
        <span className="hidden truncate text-[0.9375rem] sm:block">{entry.author}</span>
        <span className="min-w-0">
          <span className="block font-medium">{entry.subject}</span>
          <span className="block text-[0.875rem] text-ink-2 italic">
            <Prose text={entry.summary} />
          </span>
          <span className="block text-[0.8125rem] text-ink-3 sm:hidden">
            {day(entry.at)} · {entry.author}
          </span>
        </span>
        <ChevronDown aria-hidden="true" className={"mt-1 size-4 text-ink-3 transition-transform group-hover:text-foreground " + (open ? "rotate-180" : "")} />
      </button>
      {open ? (
        <div id={panel} className="grid gap-4 pb-4 pl-0 sm:pl-[6.75rem]">
          {entry.body ? <p className="max-w-[68ch] text-[0.9375rem] whitespace-pre-line text-ink-2">{entry.body.split("\n\n")[0]}</p> : null}
          {entry.readable ? <DiffList changes={entry.changes} /> : <p className="text-ink-3 italic">{entry.summary}</p>}
        </div>
      ) : null}
    </li>
  );
}

/** Every commit that touched the rulebook, newest first. */
export function History() {
  const { data, stamped } = useDash();
  const history = data?.current?.history ?? [];
  const [all, setAll] = useState(false);
  const shown = all ? history : history.slice(0, 8);
  return (
    <Section
      id="history"
      title={<span className="display text-[1.375rem] leading-none font-semibold">Changelog</span>}
      meta={<p className="text-[0.875rem] text-ink-2">Every commit that changed the rulebook, newest first.</p>}
      folded={`${history.length} ${history.length === 1 ? "change" : "changes"}${history[0] ? `, last on ${day(history[0].at)}` : ""}`}
    >
        {history.length ? (
          <>
            <div className="label hidden grid-cols-[6rem_7.5rem_9rem_minmax(0,1fr)_1rem] gap-x-3 border-b border-frame pb-1.5 sm:grid">
              <span>Rev</span>
              <span>Date</span>
              <span>By</span>
              <span>Description</span>
            </div>
            <ol>
              {shown.map((entry) => (
                <Revision key={entry.commit} entry={entry} fresh={entry.commit === stamped} number={history.length - history.indexOf(entry)} />
              ))}
            </ol>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              {history.length > shown.length ? (
                <Button variant="outline" size="sm" onClick={() => setAll(true)}>
                  Show all {history.length}
                </Button>
              ) : null}
              <Button variant="ghost" size="sm" asChild>
                <a href="/api/trail.tsv" download>
                  <Download /> Decision trail (TSV)
                </a>
              </Button>
            </div>
          </>
        ) : (
          <p className="text-ink-2 italic">No commit has touched the rulebook yet. Accepting the first draft writes revision one.</p>
        )}
    </Section>
  );
}

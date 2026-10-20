import { useState } from "react";
import { ChevronDown } from "lucide-react";

const KEY = "wh-collapsed";

function remembered(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}

/** A labelled band of the sheet: the heading in the left margin column, the
 * content beside it. A band with `collapsible` folds to one line, and the
 * choice is remembered in this browser. */
export function Section({
  id,
  title,
  meta,
  folded,
  collapsible = true,
  children,
}: {
  id: string;
  title: React.ReactNode;
  /** Shown under the heading: the hint and the count. */
  meta?: React.ReactNode;
  /** The one line that stands in for the content when it is folded. */
  folded?: React.ReactNode;
  collapsible?: boolean;
  children: React.ReactNode;
}) {
  const [closed, setClosed] = useState(() => collapsible && remembered().has(id));
  const toggle = () => {
    const next = !closed;
    setClosed(next);
    try {
      const set = remembered();
      if (next) set.add(id);
      else set.delete(id);
      localStorage.setItem(KEY, JSON.stringify([...set]));
    } catch {
      /* the fold holds for this visit */
    }
  };
  const content = "section-" + id;
  return (
    <section id={id} className="grid gap-2 lg:grid-cols-[10rem_minmax(0,1fr)] lg:gap-8">
      <div>
        {collapsible ? (
          <h2>
            <button
              type="button"
              aria-expanded={!closed}
              aria-controls={content}
              onClick={toggle}
              className="group flex cursor-pointer items-start gap-1.5 text-left"
            >
              <ChevronDown
                aria-hidden="true"
                className={"mt-[0.3rem] size-4 shrink-0 text-ink-3 transition-transform group-hover:text-foreground " + (closed ? "-rotate-90" : "")}
              />
              <span>{title}</span>
            </button>
          </h2>
        ) : (
          <h2>{title}</h2>
        )}
        {meta && !closed ? <div className="mt-2 lg:pl-[1.375rem]">{meta}</div> : null}
      </div>
      <div id={content} className="min-w-0">
        {closed ? (
          <button
            type="button"
            onClick={toggle}
            className="flex w-full cursor-pointer items-center justify-between border-y border-rule py-2 text-left text-[0.9375rem] text-ink-2 hover:text-foreground"
          >
            <span>{folded}</span>
            <span className="label">Show</span>
          </button>
        ) : (
          children
        )}
      </div>
    </section>
  );
}

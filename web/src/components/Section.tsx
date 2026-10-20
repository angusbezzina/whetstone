import { useState } from "react";
import { ChevronDown } from "lucide-react";

const KEY = "wh-collapsed";
const COLUMNS = "lg:grid-cols-[13rem_minmax(0,1fr)] lg:gap-x-8";
const EASE = "duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]";

function remembered(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}

/** A labelled band of the sheet: the heading in the left margin column, the
 * content beside it. A band with `collapsible` folds to its heading row, and
 * the choice is remembered in this browser. */
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
  /** Shown under the heading while the band is open. */
  meta?: React.ReactNode;
  /** The one line beside the heading that sums up the content. */
  folded?: React.ReactNode;
  collapsible?: boolean;
  children: React.ReactNode;
}) {
  const [closed, setClosed] = useState(() => collapsible && remembered().has(id));
  // The body clips only while it is folded or moving, so focus rings and
  // overhanging marks show once it has opened.
  const [moving, setMoving] = useState(false);
  const toggle = () => {
    const next = !closed;
    setClosed(next);
    setMoving(true);
    try {
      const set = remembered();
      if (next) set.add(id);
      else set.delete(id);
      localStorage.setItem(KEY, JSON.stringify([...set]));
    } catch {
      /* the fold holds for this visit */
    }
  };

  if (!collapsible) {
    return (
      <section id={id} className={"grid gap-2 " + COLUMNS}>
        <h2>{title}</h2>
        <div className="min-w-0">{children}</div>
      </section>
    );
  }

  const content = "section-" + id;
  return (
    <section id={id} className={"border-y transition-colors " + EASE + (closed ? " border-rule" : " border-primary")}>
      <h2>
        <button
          type="button"
          aria-expanded={!closed}
          aria-controls={content}
          onClick={toggle}
          className={"group flex w-full cursor-pointer items-center gap-x-8 py-3 text-left lg:grid " + COLUMNS}
        >
          <span className="flex items-center gap-1.5 whitespace-nowrap">
            <ChevronDown
              aria-hidden="true"
              className={"size-4 shrink-0 text-ink-3 transition-[transform,color] group-hover:text-foreground " + EASE + (closed ? " -rotate-90" : "")}
            />
            {title}
          </span>
          <span className="flex min-w-0 flex-1 items-center justify-between gap-4">
            <span className="hidden truncate font-serif text-[0.9375rem] text-ink-2 sm:inline">{folded}</span>
            <span className="label ml-auto transition-colors group-hover:text-primary">{closed ? "Show" : "Hide"}</span>
          </span>
        </button>
      </h2>
      <div
        id={content}
        inert={closed}
        onTransitionEnd={(event) => event.target === event.currentTarget && setMoving(false)}
        className={"grid transition-[grid-template-rows] " + EASE + (closed ? " grid-rows-[0fr]" : " grid-rows-[1fr]")}
      >
        <div className={"min-h-0" + (closed || moving ? " overflow-hidden" : "")}>
          <div className={"grid gap-2 pt-1 pb-6 " + COLUMNS}>
            <div>{meta ? <div className="lg:pl-[1.375rem]">{meta}</div> : null}</div>
            <div className="min-w-0">{children}</div>
          </div>
        </div>
      </div>
    </section>
  );
}

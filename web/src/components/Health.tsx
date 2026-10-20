import { Check, ChevronDown, CircleAlert, CircleX } from "lucide-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { useDash } from "@/state";
import { Command, Prose } from "./marks";

/** One line per problem: the gates, the CI check, the file, coverage. A
 * single line says so when nothing is wrong. */
export function Health() {
  const { data } = useDash();
  const rulebook = data?.rulebook;
  const health = data?.current?.health;
  const items = [...(health?.items ?? [])];
  if (rulebook?.state === "invalid") {
    items.unshift({ kind: "rulebook", tone: "fail", text: rulebook.detail ?? "The rulebook file cannot be read.", fix: null, detail: [] });
  }
  if (!items.length && !health?.healthy) return null;
  return (
    <section aria-label="Health" className="border-b border-rule px-4 py-2 sm:px-6">
      {items.length ? (
        <ul className="grid grid-cols-[minmax(0,1fr)] gap-1.5">
          {items.map((item, index) => (
            <li key={item.kind + index}>
              <Collapsible>
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[0.9375rem]">
                  {item.tone === "fail" ? (
                    <CircleX className="size-4 shrink-0 text-struck" aria-label="Problem" />
                  ) : (
                    <CircleAlert className="size-4 shrink-0 text-struck" aria-label="Warning" />
                  )}
                  <span className="min-w-0 flex-1 basis-48">
                    <Prose text={item.text} />
                  </span>
                  {item.fix ? <Command text={item.fix} /> : null}
                  {item.detail.length ? (
                    <CollapsibleTrigger className="group label inline-flex cursor-pointer items-center gap-1 hover:text-foreground">
                      {item.detail.length} {item.detail.length === 1 ? "detail" : "details"}
                      <ChevronDown className="size-3 transition-transform group-data-[state=open]:rotate-180" />
                    </CollapsibleTrigger>
                  ) : null}
                </div>
                {item.detail.length ? (
                  <CollapsibleContent>
                    <ul className="mt-1 mb-1 ml-7 grid gap-0.5 text-[0.8125rem] text-ink-2">
                      {item.detail.map((line) => (
                        <li key={line}>
                          <code>{line}</code>
                        </li>
                      ))}
                    </ul>
                  </CollapsibleContent>
                ) : null}
              </Collapsible>
            </li>
          ))}
        </ul>
      ) : (
        <p className="flex items-center gap-3 text-[0.9375rem] text-ink-2">
          <Check className="size-4 shrink-0 text-foreground" aria-hidden="true" />
          {health?.healthy}
        </p>
      )}
    </section>
  );
}

import { useEffect, useLayoutEffect, useRef } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Health } from "@/components/Health";
import { History } from "@/components/History";
import { Command } from "@/components/marks";
import { PendingBar } from "@/components/PendingBar";
import { Rulebook } from "@/components/Rulebook";
import { Setup } from "@/components/Setup";
import { TitleBlock } from "@/components/TitleBlock";
import { flash, plotSheet } from "@/motion";
import { DashProvider, useDash } from "@/state";

const COLUMNS = ["1", "2", "3", "4", "5", "6"];
const ROWS = ["A", "B", "C", "D"];

/** The drawing frame: an outer edge, zone ticks in the margin, and the
 * sheet inside it. */
function Sheet({ children }: { children: React.ReactNode }) {
  const root = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (root.current) plotSheet(root.current);
  }, []);
  return (
    <div ref={root} className="relative mx-auto max-w-[84rem] bg-background lg:p-6">
      {(["top", "bottom", "left", "right"] as const).map((edge) => (
        <span
          key={edge}
          data-edge={edge}
          aria-hidden="true"
          className={
            "pointer-events-none absolute bg-frame " +
            {
              top: "inset-x-0 top-0 h-px origin-left",
              bottom: "inset-x-0 bottom-0 h-px origin-right",
              left: "inset-y-0 left-0 w-px origin-top",
              right: "inset-y-0 right-0 w-px origin-bottom",
            }[edge]
          }
        />
      ))}
      <div aria-hidden="true" className="pointer-events-none absolute inset-x-6 top-0 hidden h-6 grid-cols-6 lg:grid">
        {COLUMNS.map((zone) => (
          <span key={zone} className="label flex items-center justify-center border-l border-rule first:border-l-0">
            {zone}
          </span>
        ))}
      </div>
      <div aria-hidden="true" className="pointer-events-none absolute inset-x-6 bottom-0 hidden h-6 grid-cols-6 lg:grid">
        {COLUMNS.map((zone) => (
          <span key={zone} className="label flex items-center justify-center border-l border-rule first:border-l-0">
            {zone}
          </span>
        ))}
      </div>
      <div aria-hidden="true" className="pointer-events-none absolute inset-y-6 left-0 hidden w-6 grid-rows-4 lg:grid">
        {ROWS.map((zone) => (
          <span key={zone} className="label flex items-center justify-center border-t border-rule first:border-t-0">
            {zone}
          </span>
        ))}
      </div>
      <div aria-hidden="true" className="pointer-events-none absolute inset-y-6 right-0 hidden w-6 grid-rows-4 lg:grid">
        {ROWS.map((zone) => (
          <span key={zone} className="label flex items-center justify-center border-t border-rule first:border-t-0">
            {zone}
          </span>
        ))}
      </div>
      <div className="relative border-frame lg:border">{children}</div>
    </div>
  );
}

function Notice() {
  const { notice } = useDash();
  const ref = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    if (notice) flash(ref.current);
  }, [notice]);
  return (
    <div className="pointer-events-none fixed inset-x-0 top-0 z-40 flex justify-center p-3">
      <p
        ref={ref}
        key={notice?.key}
        role="status"
        aria-live="polite"
        className={
          "max-w-md border bg-background px-3 py-2 text-[0.9375rem] " +
          (notice ? (notice.tone === "warn" ? "pointer-events-auto border-struck" : "pointer-events-auto border-frame") : "sr-only")
        }
      >
        {notice?.text}
      </p>
    </div>
  );
}

function ReadOnlyNote() {
  const { editable, loading, readOnlyReason } = useDash();
  if (editable || loading) return null;
  return (
    <p className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-rule px-4 py-2 text-[0.9375rem] text-ink-2 sm:px-6">
      <span>{readOnlyReason ?? "This tab can read the rulebook but not edit it."} To edit, run</span>
      <Command text="wh dash" />
      <span>in the repository; it opens an editing tab.</span>
    </p>
  );
}

function HealthUnlessSetup() {
  const { data } = useDash();
  return data?.current?.setup ? null : <Health />;
}

function Body() {
  const { data, error, loading } = useDash();
  const page = data?.current;
  if (loading) return <p className="px-6 py-16 text-ink-3 italic">Reading the rulebook…</p>;
  if (!page)
    return (
      <div className="px-6 py-16">
        <p className="text-[1.25rem]">The rulebook could not be read, so nothing here is claimed.</p>
        <p className="mt-2 text-ink-2">{error}</p>
      </div>
    );
  if (page.setup) return <Setup />;
  return (
    <div className="grid gap-16 px-4 py-8 sm:px-8 lg:px-10 lg:py-10">
      <Rulebook />
      <History />
    </div>
  );
}

function Colophon() {
  const { data } = useDash();
  const revision = data?.current?.header.revision?.slice(0, 7);
  return (
    <footer className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 border-t border-frame px-4 py-3 text-[0.8125rem] text-ink-3 sm:px-6">
      <span>
        This sheet is <code>whetstone/rulebook.json</code>
        {revision ? (
          <>
            {" "}
            at <code>{revision}</code>
          </>
        ) : null}
        . Agents read the same file with <code>wh dash --json</code>.
      </span>
      <span className="label">wh dash · {location.host}</span>
    </footer>
  );
}

export function App() {
  return (
    <DashProvider>
      <TooltipProvider delayDuration={250}>
        <a href="#content" className="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:bg-background focus:px-3 focus:py-2">
          Skip to the rulebook
        </a>
        <div className="min-h-screen lg:p-8">
          <Sheet>
            <TitleBlock />
            <ReadOnlyNote />
            <PendingBar />
            <HealthUnlessSetup />
            <main id="content" tabIndex={-1} className="outline-none">
              <Body />
            </main>
            <Colophon />
          </Sheet>
        </div>
        <Notice />
      </TooltipProvider>
    </DashProvider>
  );
}

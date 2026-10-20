import { useState } from "react";
import { ChevronDown } from "lucide-react";
import { RULEBOOK } from "@/api";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { useDash } from "@/state";
import { DiffList } from "./Diff";
import { Command, Delta, Prose } from "./marks";

/** The uncommitted edits to the rulebook, shown only when there are some. */
export function PendingBar() {
  const { data, editable, run, announce } = useDash();
  const pending = data?.current?.pending;
  const [open, setOpen] = useState(false);
  const [confirm, setConfirm] = useState<"accept" | "withdraw" | null>(null);
  const [why, setWhy] = useState("");
  const [busy, setBusy] = useState(false);
  if (!pending) return null;
  const noun = pending.count === 1 ? "uncommitted change" : "uncommitted changes";
  const act = async (verdict: "accept" | "withdraw") => {
    setBusy(true);
    const outcome = await run(
      verdict === "accept" ? { accept: true, rationale: why.trim() || null } : { withdraw: true },
      verdict === "accept" ? "Accepted: the rulebook was committed." : `Withdrawn: ${RULEBOOK} is as last committed.`,
    );
    setBusy(false);
    if (outcome.ok) {
      setConfirm(null);
      setWhy("");
      setOpen(false);
    } else if (outcome.needsInput) {
      // Strict mode, for one, refuses with the gaps to close.
      announce(outcome.response?.blocking_questions[0] ?? outcome.response?.summary ?? "Nothing was committed.", "warn");
    }
  };
  return (
    <section aria-labelledby="pending-title" className="border-b border-frame bg-wash">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 sm:px-6">
        <Delta className="size-4" />
        <div className="min-w-0 flex-1">
          <h2 id="pending-title" className="font-display text-[0.9375rem] font-medium tracking-[0.06em] uppercase">
            {pending.count} {noun} to <code className="normal-case tracking-normal">{RULEBOOK}</code>
          </h2>
          <p className="text-[0.9375rem] text-ink-2 italic">
            <Prose text={pending.summary} />
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="ghost" size="sm" aria-expanded={open} aria-controls="pending-diff" onClick={() => setOpen(!open)}>
            Review <ChevronDown className={"transition-transform " + (open ? "rotate-180" : "")} />
          </Button>
          {editable ? (
            <>
              <Button variant="outline" size="sm" onClick={() => { setConfirm("withdraw"); setOpen(true); }}>
                Withdraw
              </Button>
              <Button size="sm" onClick={() => { setConfirm("accept"); setOpen(true); }}>
                Accept
              </Button>
            </>
          ) : null}
        </div>
      </div>
      {open ? (
        <div id="pending-diff" className="grid gap-6 border-t border-rule bg-background px-4 py-5 sm:px-6 lg:grid-cols-[minmax(0,1fr)_22rem]">
          <DiffList changes={pending.changes} />
          <aside className="grid min-w-0 content-start gap-3">
            <div>
              <span className="label">Commit message accepting writes</span>
              <pre className="mt-1 border border-rule bg-card p-3 text-[0.75rem] leading-relaxed break-all whitespace-pre-wrap">
                {pending.message.subject}
                {why.trim() ? `\n\n${why.trim()}` : ""}
                {pending.message.body ? `\n\n${pending.message.body}` : ""}
              </pre>
            </div>
            {!editable ? (
              <p className="text-[0.9375rem] text-ink-2">
                Accept or withdraw from an editing tab, or in the terminal: <Command text="wh change --accept" />
              </p>
            ) : confirm === "accept" ? (
              <div className="grid gap-2">
                <label htmlFor="accept-why" className="label">Why (added to the commit, optional)</label>
                <Textarea id="accept-why" value={why} onChange={(event) => setWhy(event.target.value)} rows={2} />
                <p className="text-[0.875rem] text-ink-2">Commits {RULEBOOK} and nothing else. Share it the way you share any commit.</p>
                <div className="flex gap-2">
                  <Button disabled={busy} onClick={() => act("accept")}>
                    {busy ? "Committing…" : "Commit to the rulebook"}
                  </Button>
                  <Button variant="ghost" onClick={() => setConfirm(null)}>Cancel</Button>
                </div>
              </div>
            ) : confirm === "withdraw" ? (
              <div className="grid gap-2">
                <p className="text-[0.9375rem]">Withdrawing discards every change above and restores the file to its last commit.</p>
                <div className="flex gap-2">
                  <Button variant="destructive" disabled={busy} onClick={() => act("withdraw")}>
                    {busy ? "Withdrawing…" : "Discard these changes"}
                  </Button>
                  <Button variant="ghost" onClick={() => setConfirm(null)}>Cancel</Button>
                </div>
              </div>
            ) : null}
          </aside>
        </div>
      ) : null}
    </section>
  );
}

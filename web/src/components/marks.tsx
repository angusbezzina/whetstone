import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { cn } from "@/lib/utils";

/** The Whetstone mark: an isometric orange stone on an ink base. */
export function Stone({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className={className}>
      <path fill="currentColor" d="M2 14l10 5 10-5v3l-10 5-10-5z" />
      <path fill="var(--stone-top)" d="M5 10l8-4 7 4-8 4z" />
      <path fill="var(--stone-left)" d="M5 10l7 4v3l-7-4z" />
      <path fill="var(--stone-right)" d="M12 14l8-4v3l-8 4z" />
    </svg>
  );
}

/** The drawing's revision mark: a delta triangle, orange when something
 * here differs from the last commit. */
export function Delta({ className, title, number }: { className?: string; title?: string; number?: number }) {
  return (
    <svg viewBox="0 0 16 14" className={cn("size-3.5 shrink-0 text-primary", className)} role={title ? "img" : undefined} aria-hidden={title ? undefined : true} aria-label={title}>
      <path d="M8 1.2 14.8 12.8H1.2z" fill="none" stroke="currentColor" strokeWidth={number ? 1 : 1.4} strokeLinejoin="miter" />
      {number ? (
        <text x="8" y="11.2" textAnchor="middle" fill="currentColor" style={{ fontFamily: "var(--font-display)", fontSize: number > 9 ? 5 : 6.5, fontWeight: 600 }}>
          {number}
        </text>
      ) : null}
    </svg>
  );
}

/** A command a terminal would accept, with a copy button. */
export function Command({ text, className }: { text: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <span className={cn("inline-flex min-w-0 max-w-full items-stretch border border-rule bg-card", className)}>
      <code className="bare min-w-0 flex-1 overflow-x-auto px-2 py-1 text-[0.8125rem] whitespace-nowrap">{text}</code>
      <button
        type="button"
        className="inline-flex shrink-0 cursor-pointer items-center border-l border-rule px-2 text-ink-3 hover:text-foreground"
        aria-label={copied ? "Copied" : `Copy ${text}`}
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(text);
            setCopied(true);
            setTimeout(() => setCopied(false), 1400);
          } catch {
            /* the text stays selectable */
          }
        }}
      >
        {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      </button>
    </span>
  );
}

/** A field label beside its value, title-block style. */
export function Field({ label, children, className }: { label: string; children: React.ReactNode; className?: string }) {
  return (
    <div className={cn("grid gap-1 sm:grid-cols-[8.5rem_1fr] sm:gap-4", className)}>
      <dt className="label pt-[0.2rem]">{label}</dt>
      <dd className="min-w-0">{children}</dd>
    </div>
  );
}

/** Text from the server with `backticked` spans set as code. */
export function Prose({ text }: { text: string }) {
  return (
    <>
      {text.split(/(`[^`]+`)/g).map((part, index) =>
        part.startsWith("`") && part.endsWith("`") && part.length > 2 ? <code key={index}>{part.slice(1, -1)}</code> : part,
      )}
    </>
  );
}

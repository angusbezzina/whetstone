import { useLayoutEffect, useRef, useState, type ElementType } from "react";
import { cn } from "@/lib/utils";

/** Text edited where it stands: the same element, the same type, made
 * editable. Enter or leaving the field saves, Escape restores. A value
 * that differs from the last commit is redlined (`pending`). */
export function InlineText({
  value,
  onSave,
  editable,
  label,
  as: Tag = "span",
  className,
  placeholder,
  pending,
  multiline,
}: {
  value: string;
  /** Resolves to whether the edit was written; on false the text restores. */
  onSave: (next: string) => Promise<boolean>;
  editable: boolean;
  label: string;
  as?: ElementType;
  className?: string;
  placeholder?: string;
  pending?: boolean;
  /** Shift+Enter starts a new line; Enter alone still saves. */
  multiline?: boolean;
}) {
  const ref = useRef<HTMLElement>(null);
  const [busy, setBusy] = useState(false);
  const [empty, setEmpty] = useState(!value);

  // The element owns its text while it is edited; React only sets it.
  useLayoutEffect(() => {
    const element = ref.current;
    if (element && document.activeElement !== element) element.textContent = value;
    setEmpty(!value);
  }, [value, editable]);

  if (!editable) {
    return (
      <Tag className={cn(className, pending && "redline")}>
        {value || <span className="text-ink-3">{placeholder}</span>}
      </Tag>
    );
  }

  const restore = () => {
    if (ref.current) ref.current.textContent = value;
    setEmpty(!value);
  };
  const commit = async () => {
    const element = ref.current;
    if (!element || busy) return;
    const next = (element.textContent ?? "").replace(/ /g, " ").trim();
    if (next === value.trim()) {
      restore();
      return;
    }
    setBusy(true);
    const written = await onSave(next);
    setBusy(false);
    if (!written) restore();
  };

  return (
    <Tag
      ref={ref}
      role="textbox"
      aria-label={label}
      aria-multiline={multiline || undefined}
      aria-busy={busy || undefined}
      tabIndex={0}
      contentEditable={busy ? false : "plaintext-only"}
      spellCheck
      data-placeholder={placeholder}
      data-empty={empty || undefined}
      onInput={() => setEmpty(!(ref.current?.textContent ?? "").trim())}
      onKeyDown={(event: React.KeyboardEvent<HTMLElement>) => {
        if (event.key === "Escape") {
          event.preventDefault();
          restore();
          ref.current?.blur();
        } else if (event.key === "Enter" && !(multiline && event.shiftKey)) {
          event.preventDefault();
          ref.current?.blur();
        }
      }}
      onBlur={commit}
      className={cn("editable", className, pending && "redline", busy && "opacity-60")}
    />
  );
}

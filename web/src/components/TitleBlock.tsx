import { useEffect, useState } from "react";
import { Monitor, Moon, Sun } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { day, useDash } from "@/state";
import { Stone } from "./marks";

const THEME = "wh-theme";
type Theme = "system" | "light" | "dark";

function applyTheme(theme: Theme) {
  if (theme === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", theme);
  try {
    localStorage.setItem(THEME, theme);
  } catch {
    /* the choice holds for this tab */
  }
}

function useTheme(): [Theme, (theme: Theme) => void] {
  const [theme, setTheme] = useState<Theme>(() => {
    try {
      return (localStorage.getItem(THEME) as Theme) || "system";
    } catch {
      return "system";
    }
  });
  useEffect(() => applyTheme(theme), [theme]);
  return [theme, setTheme];
}

function Cell({ label, children, className, title, wrap }: { label: string; children: React.ReactNode; className?: string; title?: string; wrap?: boolean }) {
  return (
    <div data-plot className={"flex min-w-0 flex-col justify-between gap-1 bg-background px-3 py-2 " + (className ?? "")}>
      <span className="label">{label}</span>
      <span className={"min-w-0 text-[0.9375rem] leading-snug " + (wrap ? "" : "truncate")} title={title}>
        {children}
      </span>
    </div>
  );
}

export function TitleBlock() {
  const { data, editable, readOnlyReason, showIds, setShowIds } = useDash();
  const [theme, setTheme] = useTheme();
  const page = data?.current;
  const header = page?.header;
  const last = header?.last_change;
  const revision = header?.revision?.slice(0, 7);
  return (
    <header className="grid grid-cols-2 gap-px border-b border-frame bg-rule sm:grid-cols-4 lg:grid-cols-[auto_minmax(0,1fr)_minmax(0,1.25fr)_minmax(0,0.6fr)_minmax(0,1.4fr)_minmax(0,1.4fr)_auto]">
      <div data-plot className="col-span-2 flex items-center gap-3 bg-background px-3 py-2 sm:col-span-4 lg:col-span-1">
        <Stone className="size-8 text-foreground" />
        <div className="leading-none">
          <div className="font-display text-[1.0625rem] font-semibold tracking-[0.22em] uppercase">
            Whet<span className="text-struck">stone</span>
          </div>
          <div className="label mt-1">Rulebook</div>
        </div>
      </div>
      <Cell label="Project" title={header?.project}>
        <span className="font-display text-[1.0625rem] font-medium tracking-wide">{header?.project ?? "…"}</span>
      </Cell>
      <Cell label="Branch" title={header?.branch ?? undefined}>
        <code className="bare text-[0.8125rem]">{header ? (header.branch ?? "detached HEAD") : "…"}</code>
      </Cell>
      <Cell label="Revision" className="col-span-2 sm:col-span-1">
        {!header ? (
          "…"
        ) : revision ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <code className="bare text-[0.8125rem]">{revision}</code>
            </TooltipTrigger>
            <TooltipContent>The commit that last changed whetstone/rulebook.json</TooltipContent>
          </Tooltip>
        ) : (
          <span className="text-ink-3">none yet</span>
        )}
      </Cell>
      <Cell label="Contents" wrap className="col-span-2 sm:col-span-1">{header?.summary ?? "…"}</Cell>
      <Cell label="Last change" className="col-span-2 sm:col-span-1" title={last ? `${last.subject} (${last.author})` : undefined}>
        {!header ? (
          "…"
        ) : last ? (
          <>
            {day(last.at)} <span className="text-ink-3">by</span> {last.author}
          </>
        ) : (
          <span className="text-ink-3">no commit yet</span>
        )}
      </Cell>
      <div data-plot className="col-span-2 flex flex-wrap items-center justify-between gap-x-4 gap-y-2 bg-background px-3 py-2 sm:col-span-4 lg:col-span-1 lg:flex-col lg:items-end lg:justify-center">
        <Tooltip>
          <TooltipTrigger asChild>
            <Badge
              className="border-frame bg-transparent text-foreground"
              tabIndex={0}
            >
              {editable ? "Editing" : "Read-only"}
            </Badge>
          </TooltipTrigger>
          <TooltipContent className="max-w-72">
            {editable
              ? "Edits are drafts in whetstone/rulebook.json until you accept them."
              : (readOnlyReason ?? "This tab can read but not edit.") + " Run wh dash in the repository to open an editing tab."}
          </TooltipContent>
        </Tooltip>
        <div className="flex items-center gap-3">
          <label className="flex cursor-pointer items-center gap-1.5">
            <Switch checked={showIds} onCheckedChange={setShowIds} aria-label="Show ids and digests" />
            <span className="label">Ids</span>
          </label>
          <ToggleGroup
            type="single"
            value={theme}
            onValueChange={(value) => value && setTheme(value as Theme)}
            aria-label="Colour theme"
            className="gap-0"
          >
            <ToggleGroupItem value="system" aria-label="Match the system" className="size-7 min-w-7 p-0">
              <Monitor className="size-3.5" />
            </ToggleGroupItem>
            <ToggleGroupItem value="light" aria-label="Light" className="size-7 min-w-7 p-0">
              <Sun className="size-3.5" />
            </ToggleGroupItem>
            <ToggleGroupItem value="dark" aria-label="Dark" className="size-7 min-w-7 p-0">
              <Moon className="size-3.5" />
            </ToggleGroupItem>
          </ToggleGroup>
        </div>
      </div>
    </header>
  );
}

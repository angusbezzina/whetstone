import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
  downloadPack,
  importPack,
  packPlan,
  type PackImport,
  type PackPlan,
  type PackReview,
  type PackRule,
  type ReviewFile,
  type ReviewRule,
} from "@/api";
import { useDash } from "@/state";
import { Command } from "./marks";

const PORTABILITY: Record<PackRule["portability"], string> = {
  stays: "Stays here",
  brings_files: "Brings files",
  needs_tools: "Needs tools",
  scoped: "Scoped",
  portable: "Portable",
};

const STATUS: Record<ReviewRule["status"], string> = {
  ready: "Ready",
  needs_consent: "Needs consent",
  needs_mapping: "Needs a path mapping",
  not_validatable: "Won't work here",
  file_conflict: "File differs",
  held: "Already held",
};

const KINDS = ["ast", "lint", "formatter", "test", "validator"] as const;
const MAX_PACK = 1024 * 1024;

function Chip({ strong, children }: { strong?: boolean; children: React.ReactNode }) {
  return (
    <span className={"label inline-flex h-5 shrink-0 items-center border px-1.5 whitespace-nowrap " + (strong ? "border-frame text-foreground" : "border-rule text-ink-2")}>
      {children}
    </span>
  );
}

function Tick({ label, checked, disabled, onChange, children }: { label?: string; checked: boolean; disabled?: boolean; onChange: (checked: boolean) => void; children?: React.ReactNode }) {
  return (
    <label className={"flex items-start gap-2.5 " + (disabled ? "" : "cursor-pointer")}>
      <input type="checkbox" className="tick mt-1" aria-label={label} checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
      {children ? <span className="min-w-0 flex-1">{children}</span> : null}
    </label>
  );
}

function toggled(list: string[], item: string, on: boolean): string[] {
  return on ? [...new Set([...list, item])] : list.filter((value) => value !== item);
}

/** What a rule needs, in one line, from the plan's facts alone. */
function Needs({ rule }: { rule: PackRule }) {
  const parts: React.ReactNode[] = [];
  if (rule.serves) parts.push(<>serves {rule.serves.statement.replace(/[.]+$/, "")}</>);
  if (rule.files.length) parts.push(<>brings {rule.files.map((file, index) => <span key={file}>{index ? ", " : ""}<code>{file}</code></span>)}</>);
  if (rule.tools.length) parts.push(<>needs {rule.tools.map((tool, index) => <span key={tool}>{index ? ", " : ""}<code>{tool}</code></span>)}</>);
  if (rule.paths.length) parts.push(<>{rule.paths.length} path{rule.paths.length === 1 ? "" : "s"} here</>);
  if (!parts.length) return <>Needs nothing more.</>;
  return <>{parts.map((part, index) => <span key={index}>{index ? "; " : ""}{part}</span>)}.</>;
}

function ShareOut() {
  const { announce } = useDash();
  const [plan, setPlan] = useState<PackPlan | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const [keep, setKeep] = useState<string[]>([]);
  const [extra, setExtra] = useState<string[]>([]);
  const [strength, setStrength] = useState("all");
  const [kind, setKind] = useState("all");
  const [name, setName] = useState("");
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    packPlan()
      .then((response) => (response.state === "success" ? setPlan(response.data) : setFailure(response.summary)))
      .catch((error: Error) => setFailure(error.message));
  }, []);
  const rules = plan?.rules ?? [];
  const shown = rules.filter((rule) => (strength === "all" || rule.strength === strength) && (kind === "all" || rule.enforcer === kind));
  const picked = rules.filter((rule) => chosen.includes(rule.id));
  const principles = new Set(picked.flatMap((rule) => (rule.serves ? [rule.serves.id] : [])));
  const files = new Set([...picked.flatMap((rule) => rule.files), ...extra.filter((file) => picked.some((rule) => rule.suggested_files.includes(file)))]);
  const leaving = shown.filter((rule) => rule.can_leave).map((rule) => rule.id);
  const allShown = leaving.length > 0 && leaving.every((id) => chosen.includes(id));
  const download = async () => {
    setBusy(true);
    try {
      await downloadPack({
        only: chosen,
        keep_paths: keep.filter((id) => chosen.includes(id)),
        with_files: [...files].filter((file) => extra.includes(file)),
        name: name.trim() || undefined,
        version: version.trim() || undefined,
      });
      announce(`The pack holds ${picked.length} rule${picked.length === 1 ? "" : "s"}, ${principles.size} principle${principles.size === 1 ? "" : "s"} and ${files.size} file${files.size === 1 ? "" : "s"}.`);
    } catch (error) {
      announce((error as Error).message, "warn");
    } finally {
      setBusy(false);
    }
  };
  if (failure) return <p className="text-ink-2">{failure}</p>;
  if (!plan) return <p className="text-ink-3 italic">Reading what each rule needs…</p>;
  return (
    <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_20rem]">
      <div className="min-w-0">
        <div className="mb-3 flex flex-wrap items-center gap-x-5 gap-y-2">
          <ToggleGroup type="single" value={strength} onValueChange={(value) => value && setStrength(value)} aria-label="Filter by strength" className="gap-0">
            {[["all", "All"], ["must", "Hard"], ["should", "Soft"]].map(([value, label]) => (
              <ToggleGroupItem key={value} value={value} className="display h-7 px-2 text-[0.6875rem]">{label}</ToggleGroupItem>
            ))}
          </ToggleGroup>
          <ToggleGroup type="single" value={kind} onValueChange={(value) => value && setKind(value)} aria-label="Filter by enforcer" className="flex-wrap gap-0">
            {["all", ...KINDS].map((value) => (
              <ToggleGroupItem key={value} value={value} className="display h-7 px-2 text-[0.6875rem]">{value === "all" ? "Every check" : value}</ToggleGroupItem>
            ))}
          </ToggleGroup>
          <Button variant="ghost" size="sm" className="ml-auto" disabled={!leaving.length} onClick={() => setChosen(allShown ? chosen.filter((id) => !leaving.includes(id)) : [...new Set([...chosen, ...leaving])])}>
            {allShown ? "Clear shown" : "Select shown"}
          </Button>
        </div>
        {shown.length ? (
          <ul className="border-t border-frame">
            {shown.map((rule) => {
              const on = chosen.includes(rule.id);
              return (
                <li key={rule.id} className="border-b border-rule py-2.5">
                  <Tick checked={on} disabled={!rule.can_leave} onChange={(checked) => setChosen(toggled(chosen, rule.id, checked))}>
                    <span className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                      <span className={"min-w-0 text-[1.0625rem] leading-snug font-medium " + (rule.can_leave ? "" : "text-ink-3")}>{rule.statement}</span>
                      <span className="flex gap-1.5">
                        <Chip strong>{rule.enforcer}</Chip>
                        <Chip>{PORTABILITY[rule.portability]}</Chip>
                      </span>
                    </span>
                    <span className="mt-0.5 block text-[0.875rem] text-ink-2">{rule.can_leave ? <Needs rule={rule} /> : rule.reason ? `It stays: ${rule.reason}.` : null}</span>
                  </Tick>
                  {on && (rule.paths.length > 0 || rule.suggested_files.length > 0) ? (
                    <div className="mt-2 ml-6.5 grid gap-1.5 border-l border-rule pl-3 text-[0.875rem]">
                      {rule.paths.length ? (
                        <Tick checked={keep.includes(rule.id)} onChange={(checked) => setKeep(toggled(keep, rule.id, checked))}>
                          Keep its paths: {rule.paths.map((path, index) => <span key={path}>{index ? ", " : ""}<code>{path}</code></span>)}. Left off, it covers every file there.
                        </Tick>
                      ) : null}
                      {rule.suggested_files.length ? (
                        <details>
                          <summary className="label cursor-pointer py-0.5">
                            {rule.suggested_files.length} file{rule.suggested_files.length === 1 ? "" : "s"} its scripts mention
                          </summary>
                          <p className="mt-1 text-ink-2">Include what the script reads. Some may be this repository's own state.</p>
                          <div className="mt-1 grid gap-1">
                            {rule.suggested_files.map((file) => (
                              <Tick key={file} checked={extra.includes(file)} onChange={(checked) => setExtra(toggled(extra, file, checked))}>
                                <code>{file}</code>
                              </Tick>
                            ))}
                          </div>
                        </details>
                      ) : null}
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        ) : (
          <p className="text-ink-2 italic">No rule matches these filters.</p>
        )}
      </div>
      <aside aria-label="What the pack holds" className="self-start border border-frame lg:sticky lg:top-4">
        <dl className="grid grid-cols-3 gap-px border-b border-frame bg-rule" aria-live="polite">
          {[["Rules", picked.length], ["Principles", principles.size], ["Files", files.size]].map(([label, count]) => (
            <div key={label} className="bg-background px-3 py-2">
              <dt className="label">{label}</dt>
              <dd className="font-display text-[1.375rem] font-medium">{count}</dd>
            </div>
          ))}
        </dl>
        <div className="grid gap-3 p-3">
          <div className="grid gap-1">
            <label htmlFor="pack-name" className="label">Name</label>
            <Input id="pack-name" className="font-mono text-[0.8125rem]" placeholder={plan.name} value={name} onChange={(event) => setName(event.target.value)} />
          </div>
          <div className="grid gap-1">
            <label htmlFor="pack-version" className="label">Version</label>
            <Input id="pack-version" className="font-mono text-[0.8125rem]" placeholder={plan.version} value={version} onChange={(event) => setVersion(event.target.value)} />
          </div>
          <Button disabled={busy || !picked.length} onClick={download}>{busy ? "Exporting…" : "Export pack"}</Button>
          <p className="text-[0.875rem] text-ink-2">A pack holds what is committed. A chosen rule brings the principle it serves. Nothing here is written or changed.</p>
        </div>
      </aside>
    </div>
  );
}

function ReadFile({ file, text }: { file: ReviewFile; text: string | undefined }) {
  const kb = Math.max(1, Math.round(file.bytes / 1024));
  return (
    <li className="border-b border-rule py-2">
      <details>
        <summary className="flex cursor-pointer flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
          <code className="bare min-w-0 text-[0.8125rem] break-all">{file.path}</code>
          <span className="flex items-center gap-1.5">
            <span className="text-[0.8125rem] text-ink-3">{kb} KiB{file.executable ? ", executable" : ""}</span>
            <Chip strong={file.status === "conflict"}>{{ new: "New", present: "Already here", conflict: "File differs" }[file.status]}</Chip>
          </span>
        </summary>
        <pre className="mt-2 max-h-80 overflow-auto border border-rule bg-card p-3 font-mono text-[0.75rem] leading-relaxed whitespace-pre-wrap">{text ?? ""}</pre>
      </details>
    </li>
  );
}

type Applied = { summary: string; data: PackImport; blocking: string[] };

function Result({ result }: { result: Applied }) {
  const { data } = result;
  const list = (title: string, items: React.ReactNode[]) =>
    items.length ? (
      <div>
        <h4 className="label">{title}</h4>
        <ul className="mt-1 grid gap-1 text-[0.9375rem]">{items.map((item, index) => <li key={index}>{item}</li>)}</ul>
      </div>
    ) : null;
  const written = data.files_written ?? [];
  return (
    <section role="status" className="grid gap-4 border border-frame p-4">
      <p>{result.summary}</p>
      {result.blocking.filter((question) => question !== result.summary).map((question) => <p key={question} className="text-ink-2">{question}</p>)}
      {list("Fails today", (data.fails_today ?? []).map((item) => <><code>{item.id}</code> ({item.strength === "must" ? "blocks the gate once accepted" : "reports"}): {item.reason || "the command exited with a failure"}</>))}
      {list("Won't work here", (data.not_validatable ?? []).map((item) => <><code>{item.id}</code>: {item.reason}</>))}
      {list("Left out, a file here differs", (data.file_conflicts ?? []).map((item) => <><code>{item.id}</code>: {item.files.map((file) => <code key={file}>{file}</code>)}</>))}
      {list("Serves a principle that reads differently here", (data.principle_conflicts ?? []).map((item) => <><code>{item.rule}</code> now serves “{item.here}”, not “{item.pack}”</>))}
      {list("Serves nothing here", (data.principle_missing ?? []).map((item) => <><code>{item.rule}</code>: neither the pack nor this rulebook holds <code>{item.principle}</code></>))}
      {list("Now covers every file", (data.widened ?? []).map((id) => <code>{id}</code>))}
      {written.length ? (
        <div className="grid gap-2">
          <h4 className="label">Commit these files, then accept</h4>
          <p className="text-[0.9375rem] text-ink-2">Accepting commits the rulebook and only it, so the files go first.</p>
          <Command text={`git add ${written.join(" ")} && git commit -m "Add the files the rules run"`} />
        </div>
      ) : null}
    </section>
  );
}

function BringIn() {
  const { editable, reload, announce } = useDash();
  const inputId = useId();
  const [text, setText] = useState<string | null>(null);
  const [fileName, setFileName] = useState("");
  const [review, setReview] = useState<PackReview | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [skipped, setSkipped] = useState<string[]>([]);
  const [maps, setMaps] = useState<string[]>([]);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Applied | null>(null);

  // The file texts come from the upload itself; the server's review names
  // them. Everything here renders as plain text.
  const fileTexts = useMemo(() => {
    try {
      const parsed = JSON.parse(text ?? "{}") as { files?: { path: string; text: string }[] };
      return new Map((parsed.files ?? []).map((file) => [file.path, file.text]));
    } catch {
      return new Map<string, string>();
    }
  }, [text]);

  const runReview = useCallback(async (pack: string, mapPaths: string[]) => {
    setBusy(true);
    setProblem(null);
    try {
      const response = await importPack({ pack, mode: "review", map_paths: mapPaths });
      if (response.data?.review) {
        setReview(response.data.review);
      } else {
        setReview(null);
        setProblem(response.data?.principle_pack ? `${response.summary} Install it with wh change --import-pack in a terminal.` : response.summary);
      }
    } catch (error) {
      setReview(null);
      setProblem((error as Error).message);
    } finally {
      setBusy(false);
    }
  }, []);

  const choose = async (file: File | undefined) => {
    setResult(null);
    setConsent(false);
    setSkipped([]);
    setMaps([]);
    if (!file) return;
    if (file.size > MAX_PACK) {
      setProblem(`${file.name} is larger than a pack can be (1 MiB).`);
      return;
    }
    setFileName(file.name);
    const pack = await file.text();
    setText(pack);
    await runReview(pack, []);
  };

  const addMap = async (map: string) => {
    const next = [...new Set([...maps, map])];
    setMaps(next);
    setConsent(false);
    if (text) await runReview(text, next);
  };

  if (!editable) {
    return (
      <div className="grid gap-2">
        <p>Bringing rules in writes drafts, so it needs an editing tab. Run this in the repository:</p>
        <Command text="wh dash" />
      </div>
    );
  }

  const rules = (review?.entries.filter((entry) => entry.kind === "rule") ?? []) as ReviewRule[];
  const principles = review?.entries.filter((entry) => entry.kind === "principle") ?? [];
  const blocked = (rule: ReviewRule) => rule.status === "held" || rule.status === "file_conflict";
  const active = rules.filter((rule) => !blocked(rule) && !skipped.includes(rule.id));
  const commands = active.filter((rule) => rule.status === "needs_consent");
  const carried = new Set(commands.flatMap((rule) => rule.files));
  const writes = (review?.files ?? []).filter((file) => carried.has(file.path) && file.status === "new");
  const drafts = active.filter((rule) => rule.status === "ready" || (consent && rule.status === "needs_consent")).length;
  const needsConsent = commands.length > 0;

  const apply = async () => {
    if (!text || !review) return;
    setBusy(true);
    try {
      // An empty list means the whole pack, so with every rule unticked the
      // principles are what was chosen.
      const ruleIds = active.map((rule) => rule.id);
      const only = ruleIds.length ? ruleIds : principles.map((principle) => principle.id);
      const everything = skipped.length === 0;
      const response = await importPack({
        pack: text,
        mode: "apply",
        only: everything ? [] : only,
        map_paths: maps,
        allow_commands: consent,
      });
      setResult({ summary: response.summary, data: response.data, blocking: response.blocking_questions });
      setConsent(false);
      await reload();
      if (response.data?.review) setReview(response.data.review);
    } catch (error) {
      announce((error as Error).message, "warn");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid gap-6">
      <div className="grid gap-2 border border-frame p-4">
        <label htmlFor={inputId} className="label">A pack file</label>
        <input
          id={inputId}
          type="file"
          accept=".json,application/json"
          className="font-mono text-[0.8125rem] file:mr-3 file:cursor-pointer file:border file:border-frame file:bg-transparent file:px-3 file:py-1.5 file:font-display file:text-[0.8125rem] file:tracking-[0.08em] file:uppercase"
          onChange={(event) => choose(event.target.files?.[0])}
        />
        <p className="text-[0.875rem] text-ink-2">Reading a pack runs nothing and writes nothing. You will see every command and file first.</p>
      </div>
      {problem ? <p role="alert" className="border border-frame p-3">{problem}</p> : null}
      {busy && !review ? <p className="text-ink-3 italic">Reading {fileName}…</p> : null}
      {review ? (
        <>
          <dl className="grid grid-cols-2 gap-px border border-frame bg-rule sm:grid-cols-4">
            {[["Pack", review.pack.name], ["Version", review.pack.version], ["From", review.pack.source], ["Digest", review.pack.digest.slice(0, 19)]].map(([label, value]) => (
              <div key={label} className="min-w-0 bg-background px-3 py-2">
                <dt className="label">{label}</dt>
                <dd className="truncate text-[0.9375rem]" title={value}>{value}</dd>
              </div>
            ))}
          </dl>
          {rules.length ? (
            <section aria-label="Rules in the pack">
              <h3 className="display mb-2 text-[1.125rem] font-medium">Rules</h3>
              <ul className="border-t border-frame">
                {rules.map((rule) => (
                  <li key={rule.id} className="border-b border-rule py-2.5">
                    <Tick
                      label={`Bring in ${rule.statement}`}
                      checked={!blocked(rule) && !skipped.includes(rule.id)}
                      disabled={blocked(rule)}
                      onChange={(checked) => setSkipped(toggled(skipped, rule.id, !checked))}
                    >
                      <span className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                        <span className="min-w-0 text-[1.0625rem] leading-snug font-medium">{rule.statement}</span>
                        <span className="flex gap-1.5">
                          <Chip strong>{rule.strength === "must" ? "Hard" : "Soft"}</Chip>
                          <Chip strong>{rule.enforcer}</Chip>
                          <Chip strong={rule.status === "needs_consent" || rule.status === "file_conflict"}>{STATUS[rule.status]}</Chip>
                        </span>
                      </span>
                    </Tick>
                    <div className="mt-1 ml-6.5 grid gap-1 text-[0.875rem] text-ink-2">
                      <pre className="border border-rule bg-card px-2 py-1 font-mono text-[0.75rem] text-foreground break-all whitespace-pre-wrap">{rule.target}</pre>
                      {rule.reason ? <p>{rule.reason}</p> : null}
                      {rule.paths.length ? <p>Covers {rule.paths.map((path, index) => <span key={path}>{index ? ", " : ""}<code>{path}</code></span>)}</p> : null}
                      {rule.widened ? <p>It had paths where it came from and will cover every file here.</p> : null}
                      {rule.serves ? <p>Serves <code>{rule.serves}</code>{rule.principle_conflict ? ", which says something different here" : ""}.</p> : null}
                      {rule.principle_missing ? <p>The principle it served is in neither the pack nor this rulebook.</p> : null}
                      {rule.files.length ? <p>Needs {rule.files.map((file, index) => <span key={file}>{index ? ", " : ""}<code>{file}</code></span>)}</p> : null}
                      {rule.probe.summary ? <p>{rule.probe.summary}</p> : rule.runs_command ? <p>Not run.</p> : null}
                      {rule.suggestion ? (
                        <p>
                          <Button variant="outline" size="sm" disabled={busy} onClick={() => addMap(`${rule.suggestion!.from}=${rule.suggestion!.to}`)}>
                            Map {rule.suggestion.from} to {rule.suggestion.to}
                          </Button>
                        </p>
                      ) : null}
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}
          {principles.length ? (
            <section aria-label="Principles in the pack">
              <h3 className="display mb-2 text-[1.125rem] font-medium">Principles</h3>
              <ul className="border-t border-frame">
                {principles.map((principle) => (
                  <li key={principle.id} className="flex flex-wrap items-baseline justify-between gap-x-3 border-b border-rule py-2">
                    <span className="min-w-0">{principle.statement}</span>
                    <Chip strong={principle.status === "conflict"}>{{ ready: "Ready", held: "Already held", conflict: "Reads differently here" }[principle.status as "ready" | "held" | "conflict"]}</Chip>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}
          {review.files.length ? (
            <section aria-label="Files in the pack">
              <h3 className="display mb-2 text-[1.125rem] font-medium">Files</h3>
              <ul className="border-t border-frame">
                {review.files.map((file) => <ReadFile key={file.path} file={file} text={fileTexts.get(file.path)} />)}
              </ul>
            </section>
          ) : null}
          <section aria-label="Map a path" className="grid gap-2">
            <h3 className="label">Map a path</h3>
            <form
              className="flex flex-wrap items-end gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                if (from.trim()) addMap(`${from.trim()}=${to.trim()}`);
                setFrom("");
                setTo("");
              }}
            >
              <Input aria-label="Path in the pack" className="max-w-60 font-mono text-[0.8125rem]" placeholder="apps/web/src" value={from} onChange={(event) => setFrom(event.target.value)} />
              <Input aria-label="Path here" className="max-w-60 font-mono text-[0.8125rem]" placeholder="src" value={to} onChange={(event) => setTo(event.target.value)} />
              <Button variant="outline" size="sm" type="submit" disabled={busy || !from.trim()}>Map</Button>
            </form>
            {maps.length ? <p className="text-[0.875rem] text-ink-2">Mapped: {maps.map((map, index) => <span key={map}>{index ? ", " : ""}<code>{map}</code></span>)}</p> : null}
          </section>
          <section aria-label="Import" className="grid gap-3 border border-frame p-4">
            {needsConsent ? (
              <Tick checked={consent} onChange={setConsent}>
                I have read them. Let {commands.length} command{commands.length === 1 ? "" : "s"} run here and write {writes.length} file{writes.length === 1 ? "" : "s"}.
              </Tick>
            ) : null}
            <div className="flex flex-wrap items-center gap-3">
              <Button disabled={busy || drafts + principles.filter((principle) => principle.status === "ready").length === 0} onClick={apply}>
                {busy ? "Importing…" : needsConsent && !consent ? `Import ${drafts} draft${drafts === 1 ? "" : "s"} without commands` : `Import ${drafts} draft${drafts === 1 ? "" : "s"}`}
              </Button>
              <span className="text-[0.875rem] text-ink-2">Nothing takes force until you accept it.</span>
            </div>
          </section>
          {result ? <Result result={result} /> : null}
        </>
      ) : null}
    </div>
  );
}

/** Share rules: choose rules from this repository into a pack, or bring a
 * pack in. The transmittal view of the same sheet. */
export function ShareRules() {
  const { data } = useDash();
  // A new repository has nothing to share yet, and rules to bring in.
  const fresh = Boolean(data?.current?.setup);
  return (
    <div className="grid gap-5 px-4 py-8 sm:px-8 lg:px-10 lg:py-10">
      <div>
        <h2 className="display text-[1.375rem] leading-none font-semibold">Share rules</h2>
        <p className="mt-2 text-ink-2 italic">A pack is one file. It takes rules from one repository to another, and a person reads it before anything runs.</p>
      </div>
      <Tabs defaultValue={fresh ? "in" : "out"}>
        <TabsList variant="line" className="mb-5 h-auto w-full justify-start gap-4 border-b border-rule p-0">
          {[["out", "Share out"], ["in", "Bring in"]].map(([value, label]) => (
            <TabsTrigger key={value} value={value} className="display -mb-px flex-none rounded-none border-0 border-b-2 border-transparent px-0 pb-2 text-[0.8125rem] data-[state=active]:border-primary data-[state=active]:text-foreground">
              {label}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value="out"><ShareOut /></TabsContent>
        <TabsContent value="in" forceMount className="data-[state=inactive]:hidden"><BringIn /></TabsContent>
      </Tabs>
    </div>
  );
}

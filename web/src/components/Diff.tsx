import type { EntryChange, FieldChange } from "@/api";
import { useDash } from "@/state";

const FIELD: Record<string, string> = {
  statement: "Statement",
  rationale: "Why",
  strength: "Strength",
  enforcer: "Checked by",
  paths: "Applies to",
  exclude: "Excludes",
  mode: "Mode",
  batch: "Batched",
  examples: "Examples",
  source: "Source",
  sweep: "When broken",
  sweep_priority: "Priority",
  sweep_unsure: "Unsure areas",
};

const ACTION: Record<EntryChange["action"], string> = {
  added: "Added",
  changed: "Changed",
  retired: "Retired",
  reinstated: "Reinstated",
  removed: "Removed",
};

/** Plain words for a raw field value: must/should become blocks/reports. */
function words(field: string, value: string | null): string | null {
  if (value === null) return null;
  if (field === "strength") return value === "must" ? "hard (must)" : value === "should" ? "soft (should)" : value;
  if (field === "sweep")
    return { discuss: "discuss first", confirm_first: "fix once approved", draft_pr: "fix as a draft PR" }[value] ?? value;
  return value;
}

/** Commands, paths and patterns are literal text: set them as code. */
function literal(field: string, value: string | null) {
  return ["enforcer", "paths", "exclude", "sweep_unsure", "examples", "source"].includes(field) ? <code className="whitespace-normal">{value}</code> : value;
}

function Fields({ fields }: { fields: FieldChange[] }) {
  return (
    <table className="mt-2 w-full border-collapse text-[0.9375rem]">
      <thead className="sr-only">
        <tr>
          <th>Field</th>
          <th>Before</th>
          <th>After</th>
        </tr>
      </thead>
      <tbody>
        {fields.map((change) => (
          <tr key={change.field} className="border-t border-rule align-top">
            <th scope="row" className="label w-28 py-1.5 pr-3 text-left font-medium">
              {FIELD[change.field] ?? change.field}
            </th>
            <td className="py-1.5 pr-3">
              {change.before === null ? <span className="text-ink-3 italic">none</span> : <span className="del break-words">{literal(change.field, words(change.field, change.before))}</span>}
            </td>
            <td className="py-1.5">
              {change.after === null ? <span className="text-ink-3 italic">none</span> : <span className="ins break-words">{literal(change.field, words(change.field, change.after))}</span>}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Each entry a change touched, with its fields before and after. */
export function DiffList({ changes }: { changes: EntryChange[] }) {
  const { showIds } = useDash();
  if (!changes.length) return <p className="text-ink-3 italic">No entry changed.</p>;
  return (
    <ul className="grid gap-4">
      {changes.map((change) => (
        <li key={change.kind + change.id}>
          <div className="flex flex-wrap items-baseline gap-x-2">
            <span className="label text-struck">{ACTION[change.action]}</span>
            <span className="label">{change.kind}</span>
            {showIds && change.kind !== "mission" ? <code className="bare text-[0.75rem] text-ink-3">{change.id}</code> : null}
          </div>
          <p className={change.action === "retired" || change.action === "removed" ? "del mt-0.5" : "mt-0.5 font-medium"}>
            {change.title || (change.kind === "mission" ? "The mission" : change.id)}
          </p>
          {change.reason ? <p className="mt-0.5 text-ink-2 italic">Why: {change.reason}</p> : null}
          {change.fields.length ? <Fields fields={change.fields} /> : null}
        </li>
      ))}
    </ul>
  );
}

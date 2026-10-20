import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { change, inspect, startSession, type ChangeCommand, type DashData, type Envelope } from "@/api";

export type Outcome = { ok: boolean; needsInput: boolean; response: Envelope<Record<string, unknown>> | null; error: string | null };

type Dash = {
  data: DashData | null;
  error: string | null;
  loading: boolean;
  editable: boolean;
  readOnlyReason: string | null;
  showIds: boolean;
  setShowIds: (value: boolean) => void;
  /** Send one `wh change` and reload; the message is announced on success. */
  run: (command: ChangeCommand, success?: string) => Promise<Outcome>;
  reload: () => Promise<void>;
  announce: (text: string, tone?: "ok" | "warn") => void;
  notice: { text: string; tone: "ok" | "warn"; key: number } | null;
  /** The commit a just-accepted change produced, for the revision stamp. */
  stamped: string | null;
};

const Context = createContext<Dash | null>(null);

export function useDash(): Dash {
  const value = useContext(Context);
  if (!value) throw new Error("useDash outside DashProvider");
  return value;
}

const IDS = "wh-show-ids";

export function DashProvider({ children }: { children: ReactNode }) {
  const [data, setData] = useState<DashData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [editable, setEditable] = useState(false);
  const [readOnlyReason, setReadOnlyReason] = useState<string | null>(null);
  const [showIds, setShowIdsState] = useState(() => {
    try {
      return localStorage.getItem(IDS) === "1";
    } catch {
      return false;
    }
  });
  const [notice, setNotice] = useState<Dash["notice"]>(null);
  const [stamped, setStamped] = useState<string | null>(null);
  const counter = useRef(0);

  const announce = useCallback((text: string, tone: "ok" | "warn" = "ok") => {
    counter.current += 1;
    const key = counter.current;
    setNotice({ text, tone, key });
    setTimeout(() => setNotice((current) => (current?.key === key ? null : current)), 6000);
  }, []);

  const reload = useCallback(async () => {
    try {
      const response = await inspect();
      setData(response.data);
      setError(response.data.current ? null : response.summary);
    } catch (failure) {
      setError((failure as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    (async () => {
      const session = await startSession().catch(() => ({ editable: false, reason: null }));
      setEditable(session.editable);
      setReadOnlyReason(session.reason);
      await reload();
    })();
  }, [reload]);

  const run = useCallback(
    async (command: ChangeCommand, success?: string): Promise<Outcome> => {
      try {
        const response = await change(command);
        const ok = response.state === "success";
        const needsInput = response.state === "needs_input" || response.state === "needs_decision";
        if (ok) {
          if (command.accept && typeof response.data?.commit === "string") setStamped(response.data.commit);
          announce(success ?? response.summary);
          await reload();
        } else if (!needsInput) {
          announce(response.summary, "warn");
        }
        return { ok, needsInput, response, error: ok ? null : response.summary };
      } catch (failure) {
        const message = (failure as Error).message;
        announce(message, "warn");
        return { ok: false, needsInput: false, response: null, error: message };
      }
    },
    [announce, reload],
  );

  const setShowIds = useCallback((value: boolean) => {
    setShowIdsState(value);
    try {
      localStorage.setItem(IDS, value ? "1" : "0");
    } catch {
      /* the toggle still works for this tab */
    }
  }, []);

  const value = useMemo(
    () => ({ data, error, loading, editable, readOnlyReason, showIds, setShowIds, run, reload, announce, notice, stamped }),
    [data, error, loading, editable, readOnlyReason, showIds, setShowIds, run, reload, announce, notice, stamped],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

const FORMAT = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
const TIME = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

export function day(at: string): string {
  const date = new Date(at);
  return Number.isNaN(date.getTime()) ? at : FORMAT.format(date);
}

export function clock(at: string): string {
  const date = new Date(at);
  return Number.isNaN(date.getTime()) ? "" : TIME.format(date);
}

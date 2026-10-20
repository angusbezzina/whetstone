// One motion grammar, the plotter's: lines draw first, text follows. Every
// element is visible before its animation runs, and nothing moves under
// prefers-reduced-motion.
import { animate, createTimeline, stagger, svg } from "animejs";

export const reduced = () =>
  typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

const EASE = "outExpo";

/** The sheet's frame is plotted edge by edge, then the title block fills. */
export function plotSheet(root: HTMLElement) {
  if (reduced()) return;
  const edges = root.querySelectorAll<HTMLElement>("[data-edge]");
  edges.forEach((edge) => {
    const horizontal = edge.dataset.edge === "top" || edge.dataset.edge === "bottom";
    animate(edge, {
      [horizontal ? "scaleX" : "scaleY"]: [0, 1],
      duration: 520,
      delay: edge.dataset.edge === "top" || edge.dataset.edge === "left" ? 0 : 180,
      ease: EASE,
    });
  });
  animate(root.querySelectorAll("[data-plot]"), {
    opacity: [0, 1],
    translateY: [4, 0],
    duration: 420,
    delay: stagger(40, { start: 160 }),
    ease: EASE,
  });
}

/** Drafting and proof marks a zone mark flickers through. */
const GLYPHS = "§¶†‡※◊∆∑∂≈≠±∞⊕⊗◇◆○△▽⌖⌗⍟⎔⌀∴∵";

/** The zone marks in the margin drift: every second or so one of them
 * flickers through drafting symbols, lit orange, and settles back on its
 * label. Paused while the tab is hidden; still under reduced
 * motion. Returns the stop function. */
export function drift(root: HTMLElement) {
  if (reduced()) return () => {};
  const marks = [...root.querySelectorAll<HTMLElement>("[data-zone]")];
  const timers = new Set<number>();
  const later = (fn: () => void, ms: number) => {
    const id = window.setTimeout(() => {
      timers.delete(id);
      fn();
    }, ms);
    timers.add(id);
  };
  const shift = (mark: HTMLElement) => {
    const label = mark.dataset.zone!;
    const steps = 6 + Math.floor(Math.random() * 5);
    mark.dataset.shifting = "";
    for (let step = 0; step < steps; step++) {
      // Mostly symbols, with the label flickering through near the end.
      const glyph = step > steps / 2 && Math.random() < 0.35 ? label : GLYPHS[Math.floor(Math.random() * GLYPHS.length)];
      later(() => (mark.textContent = glyph), step * 60);
    }
    later(() => {
      mark.textContent = label;
      delete mark.dataset.shifting;
    }, steps * 60);
  };
  const tick = () => {
    if (!document.hidden && marks.length) {
      shift(marks[Math.floor(Math.random() * marks.length)]);
      if (Math.random() < 0.3) shift(marks[Math.floor(Math.random() * marks.length)]);
    }
    later(tick, 500 + Math.random() * 1000);
  };
  later(tick, 900);
  return () => {
    timers.forEach((id) => window.clearTimeout(id));
    marks.forEach((mark) => {
      mark.textContent = mark.dataset.zone!;
      delete mark.dataset.shifting;
    });
  };
}

/** Rows arrive in the order they rank: blocks first, then reports. */
export function cascade(rows: Iterable<Element>) {
  if (reduced()) return;
  animate([...rows], {
    opacity: [0, 1],
    translateX: [-6, 0],
    duration: 360,
    delay: stagger(16, { start: 240 }),
    ease: EASE,
  });
}

/** Opening a rule draws its leader line, then the callout unrolls. */
export function leader(line: SVGElement | null, callout: HTMLElement | null) {
  if (reduced() || !callout) return;
  if (line) {
    animate(svg.createDrawable(line), { draw: ["0 0", "0 1"], duration: 240, ease: "outQuad" });
  }
  animate(callout, {
    clipPath: ["inset(0 0 100% 0)", "inset(0 0 0% 0)"],
    opacity: [0.2, 1],
    duration: 320,
    delay: line ? 120 : 0,
    ease: EASE,
  });
}

/** A new revision is stamped into the table. */
export function stamp(element: Element | null) {
  if (reduced() || !element) return;
  animate(element, {
    scale: [1.35, 1],
    rotate: [-6, 0],
    opacity: [0, 1],
    duration: 480,
    ease: EASE,
  });
}

/** A notice arriving: it drops a few pixels into place. */
export function drop(element: Element | null) {
  if (reduced() || !element) return;
  animate(element, {
    translateY: [-8, 0],
    opacity: [0, 1],
    duration: 320,
    ease: "outQuad",
  });
}

/** The gate pass: a pen line runs through the stage cells, commit, push
 * then CI, and each gate the rule runs at fills as the line reaches it.
 * It says where the rule bites. Under reduced motion the cells are
 * simply filled. */
export function gatePass(root: HTMLElement) {
  const line = root.querySelector<HTMLElement>("[data-gate-line]");
  if (reduced() || !line) return;
  const cells = [...root.querySelectorAll<HTMLElement>("[data-stage]")];
  const width = root.getBoundingClientRect().width || 1;
  const lit = cells.filter((cell) => cell.dataset.runs === "true");
  lit.forEach((cell) => cell.setAttribute("data-unlit", ""));
  const timeline = createTimeline({ defaults: { ease: "outQuad" } });
  timeline.add(line, { scaleX: [0, 1], opacity: [1, 1], duration: 520, ease: "inOutQuad" });
  lit.forEach((cell) => {
    const at = ((cell.offsetLeft + 4) / width) * 520;
    timeline.call(() => cell.removeAttribute("data-unlit"), at);
  });
  timeline.add(line, { opacity: [1, 0], duration: 360 }, 760);
}

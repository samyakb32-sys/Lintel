// The compact strip: what the peek state shows (docs/ARCHITECTURE.md §5).
// One row, 32 px tall: a notice, a live activity, or the pills. The arbiter
// decides which; this draws it and says how wide the island has to be.

import type { Notice } from "../core/feature";
import { ACTIVITY_W, COMPACT_W, NOTICE_W, pixelWidth } from "../core/layout";
import { BRAND } from "../core/brand";
import { State, needsYou } from "../core/state";
import { setMeter, stripRow } from "../ui/components";
import { icon } from "../ui/icons";
import { h } from "../views/dom";
import { language } from "../i18n/i18n";
import { taskLine } from "../views/views";
import type { Shown } from "./arbiter";
import { surfaceKey, type Activity } from "./sections";

export type StripContent =
  | { kind: "notice"; shown: Shown }
  /** `rev` changes whenever the activity asked to be drawn again. `width`: the row's own, when it may ask (island/voice.ts). */
  | { kind: "activity"; activity: Activity; companions: Activity[]; rev: number; width?: number }
  | { kind: "pills" };

/** What a click on the strip (or resting on it) leads to. */
export interface StripTarget {
  /** Key of the section to open. */
  section?: string;
  /** The activity's own click, instead of opening anything. */
  onClick?: () => void;
}

/** How long content that is replaced while the strip stays up takes to leave (--d-fast). */
const SWAP_MS = 120;

function noticeRow(n: Notice): HTMLElement {
  return stripRow({
    icon: n.icon ?? (n.level != null ? "volume" : "info"),
    mark: n.mark,
    tone: n.tone,
    title: n.title,
    detail: n.detail,
    level: n.level,
    bars: n.bars,
    action: n.action ? { label: n.action.label, run: () => n.action!.run() } : undefined,
  });
}

/**
 * The time and the day in short ("09:41", "Sun 11"; Settings › Island, off by
 * default). The time as the system writes it (12 or 24 hours is the user's
 * choice in Windows), the day's name in the app's language.
 */
function clock(): [string, string] {
  const now = new Date();
  return [
    now.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" }),
    `${now.toLocaleDateString(language(), { weekday: "short" })} ${now.getDate()}`,
  ];
}

/** The focused pill (the mascot glides onto its mark) and a dot for each of the others. */
function pillsRow(): HTMLElement {
  const task = State.focusTask;
  const others = State.otherTasks.slice(0, 4);
  const grid = h("span", { class: "isl-grid2" });
  // An amber ring on the dot of a pill whose agent waits for the user.
  for (const o of others) grid.append(h("i", { class: needsYou(o) ? "w" : "", style: `--c:${o.color}` }));
  return h(
    "div",
    { class: "strip-row" },
    h("span", { class: "mark-slot", style: "width:28px;height:28px" }),
    h("b", { "data-shared": "title", text: task ? task.name : BRAND.name }),
    task ? h("span", { class: "d", text: taskLine(task).text }) : null,
    h("span", { class: "isl-gap" }),
    State.settings.stripClock ? h("span", { class: "isl-clock" }, h("b", { text: clock()[0] }), clock()[1]) : null,
    others.length > 0 ? grid : null,
  );
}

function pillsKey(): string {
  const task = State.focusTask;
  return [
    task?.id, task?.name, task ? taskLine(task).text : "",
    State.otherTasks.slice(0, 4).map((o) => `${o.color}${needsYou(o) ? "!" : ""}`).join(","),
    State.settings.stripClock ? clock().join(" ") : "",
  ].join("~");
}

function safe<T>(fn: () => T, fallback: T): T {
  try {
    return fn();
  } catch (err) {
    console.error("[strip] a feature's activity failed", err);
    return fallback;
  }
}

export class Strip {
  readonly el = h("div", { id: "strip" });
  /** Width of the island for what is shown. */
  width = COMPACT_W;
  /** The pills are up, so the mascot belongs on the strip. */
  showsPills = true;
  target: StripTarget = {};

  private id = "";
  private rev: unknown = null;
  private wrap: HTMLElement | null = null;

  /**
   * Draws `content`. The same notice kind or the same activity is updated in
   * place (no animation); anything else drops or fades in while the old row
   * fades out. `onScreen` false means nobody saw the old row: it just goes.
   */
  sync(content: StripContent, onScreen: boolean) {
    const id =
      content.kind === "notice" ? `notice:${content.shown.notice.kind}`
      : content.kind === "activity" ? `activity:${content.activity.key}`
      : "pills";
    const rev =
      content.kind === "notice" ? content.shown
      : content.kind === "activity" ? `${content.rev}~${content.width ?? ""}~${content.companions.map((c) => c.key).join("|")}`
      : pillsKey();
    if (id === this.id && rev === this.rev) return;
    const same = id === this.id && this.wrap != null;
    this.id = id;
    this.rev = rev;
    // The row is centred in the island at this width: see pixelWidth.
    this.width = pixelWidth(content.kind === "notice" ? NOTICE_W : content.kind === "activity" ? content.width ?? ACTIVITY_W : COMPACT_W);
    this.showsPills = content.kind === "pills";
    this.target = this.targetOf(content);

    if (same && content.kind === "notice" && this.patchLevel(content.shown.notice)) return;
    const kids = this.build(content);
    if (same) {
      this.wrap!.replaceChildren(...kids);
      this.wrap!.style.setProperty("--w", `${this.width}px`);
      return;
    }
    const old = this.wrap;
    if (old && onScreen) {
      old.classList.add("fx-out");
      window.setTimeout(() => old.remove(), SWAP_MS);
    } else {
      old?.remove();
    }
    this.wrap = h("div", { class: `strip-in ${content.kind === "notice" ? "fx-drop" : "fx-in"}` }, ...kids);
    this.wrap.style.setProperty("--w", `${this.width}px`);
    this.el.append(this.wrap);
  }

  private build(content: StripContent): HTMLElement[] {
    switch (content.kind) {
      case "notice":
        return [noticeRow(content.shown.notice)];
      case "activity": {
        const marks = content.companions
          .map((c) => (c.def.mark ? safe(() => c.def.mark!(), null) : null))
          .filter((m): m is HTMLElement => m != null);
        const row = safe(() => content.activity.def.render(), stripRow({ title: content.activity.def.id }));
        return [...marks, row];
      }
      case "pills":
        return [pillsRow()];
    }
  }

  /** A level that moved: the meter slides to it instead of the row being drawn again. */
  private patchLevel(n: Notice): boolean {
    const bar = this.wrap?.querySelector<HTMLElement>(".meter");
    const num = this.wrap?.querySelector<HTMLElement>(".isl-num");
    const title = this.wrap?.querySelector<HTMLElement>("b");
    const mark = this.wrap?.querySelector(".strip-row > svg");
    if (n.level == null || n.mark || n.action || !bar || !num || !title || !mark) return false;
    setMeter(bar, n.level);
    num.textContent = String(Math.round(n.level * 100));
    if (title.textContent !== n.title) title.textContent = n.title;
    mark.replaceWith(icon(n.icon ?? "volume", 14));
    return true;
  }

  private targetOf(content: StripContent): StripTarget {
    if (content.kind === "notice") {
      const { notice, feature } = content.shown;
      return notice.section ? { section: surfaceKey(feature, notice.section) } : {};
    }
    if (content.kind === "activity") {
      const { def, feature } = content.activity;
      return { onClick: def.onClick ? () => def.onClick!() : undefined, section: def.section ? surfaceKey(feature, def.section) : undefined };
    }
    return {};
  }
}

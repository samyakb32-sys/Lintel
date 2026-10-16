// The settings page of the feature: eleven small things, each a section.

import { Bridge } from "../../core/bridge";
import type { FeatureCtx } from "../../core/feature";
import { featureInfos } from "../../core/registry";
import { tn } from "../../i18n/i18n";
import { btn, field, row, toggle } from "../../ui/components";
import { h } from "../../views/dom";
import { applyPreset, presetChooser } from "./presets";
import { privacySection } from "./privacy";

type Reply = { error?: string } & Record<string, unknown>;

const section = (title: string, ...kids: (Node | null)[]) => h("section", {}, h("h2", { class: "section-title", text: title }), ...kids);
const note = (text: string, tone = "") => h("p", { class: `hint ${tone ? `f-misc-late-${tone}` : ""}`.trim(), text });

/** A select as the product draws one. */
function pick<T extends string | number>(options: [T, string][], value: T, onChange: (v: string) => void): HTMLElement {
  const select = h("select", {}, ...options.map(([v, label]) => h("option", { value: String(v), text: label })));
  select.value = String(value);
  select.addEventListener("change", () => onChange(select.value));
  return h("label", { class: "field" }, select);
}

export function settingsBody(cx: FeatureCtx): HTMLElement {
  const t = cx.t;
  const root = h("div", { class: "f-misc-late-page" });
  const call = async (method: string, args?: object) => (await cx.call<Reply>(method, args)) ?? ({ error: t("That did not work. Try again.") } as Reply);

  // ── Start with a bundle ────────────────────────────────────────────────────
  const presets = section(
    t("Start with a bundle"),
    note(t("A bundle turns on a set of features and turns the others off. You are asked before anything changes, and every switch stays yours.")),
    presetChooser(featureInfos(), async (p) => {
      const failed = await applyPreset(p, featureInfos());
      presets.append(note(failed.length ? t("Done, except: {names}", { names: failed.join(", ") }) : t("Done."), failed.length ? "warn" : "ok"));
    }, t("Apply")),
  );

  // ── Backup ─────────────────────────────────────────────────────────────────
  const backupOut = h("div", { class: "f-misc-late-out" });
  const showChanges = (file: string, plan: { changes: { path: string; from: unknown; to: unknown }[]; skipped: number }) => {
    backupOut.replaceChildren();
    if (!plan.changes.length) {
      backupOut.append(note(t("Nothing in this file differs from your settings.")));
      return;
    }
    const shown = plan.changes.slice(0, 40);
    const fmt = (v: unknown) => (v === null || v === undefined ? t("not set") : typeof v === "string" ? v : JSON.stringify(v));
    backupOut.append(
      note(tn("Importing changes {count} setting:", "Importing changes {count} settings:", plan.changes.length, { count: plan.changes.length })),
      h("div", { class: "f-misc-late-diff" }, ...shown.map((c) => h("div", { class: "f-misc-late-diffrow" },
        h("code", { text: c.path }), h("span", { class: "from", text: fmt(c.from) }), h("span", { text: "→" }), h("span", { class: "to", text: fmt(c.to) })))),
      plan.changes.length > shown.length ? note(tn("and {count} more", "and {count} more", plan.changes.length - shown.length, { count: plan.changes.length - shown.length })) : "",
      plan.skipped ? note(tn("{count} entry in the file was left out (keys, other machines' state, or values that do not fit).", "{count} entries in the file were left out (keys, other machines' state, or values that do not fit).", plan.skipped, { count: plan.skipped })) : "",
      note(t("A dated copy of your current settings is kept first."), "ok"),
      h("div", { class: "f-misc-late-actions" },
        btn(t("Import"), "primary", async () => {
          const r = await call("backup_apply", { path: file });
          if (r.error) return void backupOut.replaceChildren(note(r.error, "err"));
          const boot = await Bridge.boot();
          const core = (r.core ?? {}) as Record<string, unknown>;
          if (boot && Object.keys(core).length) await Bridge.saveSettings({ ...boot.settings, ...core } as typeof boot.settings);
          for (const [id, on] of Object.entries((r.enabled ?? {}) as Record<string, boolean>)) await Bridge.featureSetEnabled(id, on).catch(() => {});
          backupOut.replaceChildren(note(t("Imported. Your previous settings are kept at {path}.", { path: String(r.copy) }), "ok"));
        }),
        btn(t("Cancel"), "ghost", () => backupOut.replaceChildren()),
      ),
    );
  };
  const backup = section(
    t("Back up your settings"),
    note(t("This saves settings only. Keys and tokens stay safely in Windows and are never written to the file. Whether the app starts with Windows and which agents are connected also stays on this computer.")),
    h("div", { class: "f-misc-late-actions" },
      btn(t("Export"), "secondary", async () => {
        const r = await call("backup_export");
        backupOut.replaceChildren(
          r.error ? note(r.error, "err") : note(t("Saved to {path}", { path: String(r.path) }), "ok"),
          r.error ? "" : h("div", { class: "f-misc-late-actions" }, btn(t("Show in folder"), "ghost", () => void call("reveal", { path: r.path }))),
        );
      }),
      btn(t("Import…"), "secondary", async () => {
        const picked = await call("backup_pick");
        if (!picked.path) return;
        const plan = await call("backup_preview", { path: picked.path });
        if (plan.error) return void backupOut.replaceChildren(note(String(plan.error), "err"));
        showChanges(String(picked.path), plan as never);
      }),
    ),
    backupOut,
  );

  // ── Demo ───────────────────────────────────────────────────────────────────
  const demoState = note("");
  const demoBtn = btn(t("Play the demo"), "secondary", async () => {
    const r = await call("demo");
    if (r.error) demoState.textContent = String(r.error);
  });
  const paintDemo = (on: boolean, step?: number, total?: number) => {
    demoBtn.firstElementChild!.textContent = on ? t("Stop the demo") : t("Play the demo");
    demoState.textContent = on && total ? t("Step {n} of {total}", { n: step ?? 0, total, count: total }) : "";
  };
  cx.on<{ on: boolean; step?: number; total?: number }>("demo", (e) => paintDemo(e.on, e.step, e.total));
  void call("demo_state").then((r) => paintDemo(r.on === true));
  const demo = section(
    t("Demo"),
    note(t("Shows a pretend coding session on the island: steps, a code change, a permission request, a question and a second agent. Nothing really runs. The two cards wait for you like real ones.")),
    h("div", { class: "f-misc-late-actions" }, demoBtn, demoState),
  );

  // ── Cleaning mode ──────────────────────────────────────────────────────────
  const cleanOut = note("");
  const cleaning = section(
    t("Cleaning mode"),
    note(t("Locks the keyboard and mouse wheel so you can wipe them. The pointer and clicks still work. It ends when time is up, after you press Esc five times, or with the Unlock button. Ctrl+Alt+Del and Win+L are Windows' own and still work.")),
    row(t("Unlocks by itself after"), pick<number>(
      [[30, t("30 seconds")], [60, t("1 minute")], [120, t("2 minutes")], [300, t("5 minutes")], [600, t("10 minutes")]], 120,
      (v) => void cx.settings.patch({ cleaningSeconds: Number(v) }),
    )),
    h("div", { class: "f-misc-late-actions" },
      btn(t("Lock the keyboard"), "primary", async () => {
        const r = await call("cleaning_start");
        cleanOut.textContent = r.error ? String(r.error) : "";
      }),
      cleanOut,
    ),
  );
  void cx.settings.get().then((s) => {
    const sel = cleaning.querySelector("select");
    if (sel && s.cleaningSeconds) sel.value = String(s.cleaningSeconds);
  });

  // ── Watch ──────────────────────────────────────────────────────────────────
  const watchOut = note("");
  const winPick = h("select", {});
  const condPick = pick<string>([["changes", t("Changes")], ["reaches", t("Reaches a number")], ["contains", t("Shows these words")]], "changes", () => paintCond());
  const value = field({ placeholder: "" });
  const paintCond = () => {
    const c = condPick.querySelector("select")!.value;
    value.style.display = c === "changes" ? "none" : "";
    value.input.placeholder = c === "reaches" ? t("For example 80") : t("The words to wait for");
  };
  const loadWindows = async () => {
    const list = ((await cx.call<{ hwnd: number; title: string; app: string }[]>("watch_windows")) ?? []);
    winPick.replaceChildren(...list.map((w) => h("option", { value: String(w.hwnd), text: `${w.app} — ${w.title}`.slice(0, 70) })));
  };
  void loadWindows();
  paintCond();
  const watching = (on: boolean, headline?: string | null, title?: string) => {
    watchOut.textContent = on ? t("Watching {title}: {headline}", { title: title ?? "", headline: headline ?? "…" }) : "";
  };
  cx.on<{ on: boolean; headline?: string | null; matched?: boolean; error?: string | null; title?: string }>("watch", (e) => {
    if (e.error) watchOut.textContent = e.error;
    else if (e.matched) watchOut.textContent = t("It happened: {headline}", { headline: e.headline ?? "" });
    else watching(e.on, e.headline, e.title);
  });
  const watch = section(
    t("Watch a window"),
    note(t("Reads a window from time to time and shows what it says on the island, such as a percentage, a time or a number. You get a notice when your condition is met. The window can be behind others but not minimised.")),
    row(t("Window"), winPick),
    row(t("Tell me when it"), condPick),
    h("div", { class: "f-misc-late-wide" }, value),
    h("div", { class: "f-misc-late-actions" },
      btn(t("Watch"), "primary", async () => {
        const c = condPick.querySelector("select")!.value;
        const r = await call("watch_start", { hwnd: Number(winPick.value), cond: c, text: value.input.value, number: value.input.value });
        if (r.error) watchOut.textContent = String(r.error);
      }),
      btn(t("Stop"), "ghost", () => void call("watch_stop")),
      btn(t("Refresh the windows"), "ghost", () => void loadWindows()),
    ),
    watchOut,
  );

  // ── Explorer ───────────────────────────────────────────────────────────────
  const flags = { pasteImage: false, shelfSelection: false };
  const explorer = section(t("Explorer"), note(t("Both are off until you turn them on. Neither adds anything to Explorer's menus.")));
  const explorerRows = h("div", {});
  explorer.append(explorerRows);
  void cx.settings.get().then((s) => {
    Object.assign(flags, { pasteImage: s.pasteImage === true, shelfSelection: s.shelfSelection === true });
    explorerRows.replaceChildren(
      row(t("Paste a copied picture as a file"), toggle(flags.pasteImage, (on) => void cx.settings.patch({ pasteImage: on })),
        t("Press Ctrl+V in a folder or on the desktop to save a copied picture as a PNG file named Pasted_Image_ plus the time. Everywhere else, and for anything that is not a picture, Ctrl+V works as usual.")),
      row(t("Explorer selection to the shelf"), toggle(flags.shelfSelection, (on) => void cx.settings.patch({ shelfSelection: on })),
        t("A shortcut that adds the files selected in the Explorer window in front to the shelf. Choose its keys under Shortcuts.")),
    );
  });

  // ── Terminal tab ───────────────────────────────────────────────────────────
  const terminal = section(
    t("Open terminal"),
    note(t("When an agent runs in Windows Terminal, Open terminal switches to its tab, found by its folder or title. A tab that matches nothing is left alone. A Terminal running as administrator cannot be read from here.")),
  );

  // ── Reset and uninstall ────────────────────────────────────────────────────
  const resetOut = h("div", { class: "f-misc-late-out" });
  const PART: Record<string, [string, string]> = {
    startup: [t("Startup entry"), t("Stops the app from starting with Windows.")],
    secrets: [t("Saved keys"), t("The keys and tokens in the Windows Credential Manager.")],
    features: [t("What features keep"), t("History, caches and files the features saved.")],
    inbox: [t("Dropped files"), t("The copies of files dropped on the island.")],
    hooks: [t("Agent connections"), t("Removes METEOR V1's entries from the agents' settings, saving a dated copy of each file first, then removes the helper program.")],
    config: [t("Your settings"), t("Settings, shortcuts and backups. Removed after the app has quit.")],
  };
  const bytes = (n: number) => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${Math.round(n / 1024)} KB` : `${(n / 1048576).toFixed(1)} MB`);
  const showReset = async (uninstall: boolean) => {
    const plan = (await call("reset_plan", { uninstall })) as { items?: { id: string; present: boolean; detail?: string; bytes?: number; count?: number }[] };
    const chosen = new Set<string>();
    const rows = (plan.items ?? []).filter((i) => i.present).map((i) => {
      chosen.add(i.id);
      const [name, about] = PART[i.id] ?? [i.id, ""];
      const where = i.id === "secrets" ? tn("{count} key", "{count} keys", i.count ?? 0, { count: i.count ?? 0 }) : [i.detail, i.bytes ? bytes(i.bytes) : ""].filter(Boolean).join(" · ");
      return row(name, toggle(true, (on) => (on ? chosen.add(i.id) : chosen.delete(i.id))), `${about} ${where}`.trim());
    });
    resetOut.replaceChildren(
      rows.length ? note(uninstall ? t("This removes everything below and then quits. Windows then removes the program itself.") : t("This removes what is ticked. The app stays installed and keeps running.")) : note(t("There is nothing to remove.")),
      ...rows,
      rows.length ? h("div", { class: "f-misc-late-actions" },
        btn(uninstall ? t("Remove and quit") : t("Remove"), "danger", async () => {
          const r = await call("reset_apply", { ids: [...chosen], uninstall });
          resetOut.replaceChildren(...Object.entries(r).map(([id, v]) =>
            note(`${PART[id]?.[0] ?? id}: ${typeof v === "string" ? v : (v as { error: string }).error}`, typeof v === "string" ? "ok" : "err")));
          if (uninstall) void call("open_settings", { page: "appsfeatures" });
        }),
        btn(t("Cancel"), "ghost", () => resetOut.replaceChildren()),
      ) : "",
    );
  };
  const reset = section(
    t("Reset or remove"),
    note(t("Nothing is removed until you have seen the list and pressed the button.")),
    h("div", { class: "f-misc-late-actions" },
      btn(t("Reset…"), "secondary", () => void showReset(false)),
      btn(t("Uninstall…"), "ghost", () => void showReset(true)),
      btn(t("Apps in Windows Settings"), "ghost", () => void call("open_settings", { page: "appsfeatures" })),
    ),
    resetOut,
  );

  root.append(presets, backup, demo, cleaning, watch, explorer, terminal, privacySection((page) => void call("open_settings", { page })), reset);
  root.append(note(t("Shortcuts for the demo, cleaning mode and the Explorer selection can be set under Shortcuts.")));
  return root;
}

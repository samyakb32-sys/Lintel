// Hub — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// One window: a tab for every agent, and beside it the hub panel, where a
// hand-off waits for the user's OK before it is delivered to another agent.
// Rust owns the bus (crates/hub); this file only draws it. Nothing runs at import.

import "./style.css";
import { defineFeature, type FeatureCtx } from "../../core/feature";
import { h } from "../../views/dom";
import { icon } from "../../ui/icons";
import { btn, card, field, iconBtn, row, toggle } from "../../ui/components";
import { Bridge } from "../../core/bridge";

const P = "f-hub-";

interface Agent { id: string; name: string; kind: "user" | "cli" | "web"; caps: string[] }
interface Task { id: number; channel: string; from: string; to: string; need: string; text: string; status: "proposed" | "sent" | "done" | "skipped"; result: string | null; forward: "waiting" | "sent" | "skipped" | null }
interface Skill { id: number; title: string; body: string; source: string; tags: string[] }
interface Entry { name: string; path: string; dir: boolean; size: number; blocked: boolean }
interface Listing { path: string; entries: Entry[]; truncated: boolean }
interface Attachment { key: string; title: string; body: string }
interface Ask { id: number; task: number; agent: string; kind: "read" | "list" | "write"; path: string; status: "pending" | "allowed" | "denied"; auto: boolean; content: string | null; replaces: boolean }
interface Msg { id: number; channel: string; from: string; text: string; at: number }

// The agents offered before an adapter reports one. `tone` picks a token colour in style.css.
const KNOWN = [
  { id: "chatgpt", name: "ChatGPT", tone: 1, url: "https://chatgpt.com", adapter: true },
  { id: "claude", name: "Claude", tone: 2, url: "https://claude.ai", adapter: true },
  { id: "gemini", name: "Gemini", tone: 3, url: "https://gemini.google.com", adapter: true },
  { id: "grok", name: "Grok", tone: 4, url: "https://grok.com", adapter: true },
];
const tone = (id: string) => KNOWN.find((k) => k.id === id)?.tone ?? 1 + ([...id].reduce((n, c) => n + c.charCodeAt(0), 0) % 4);

function avatar(a: { id: string; name: string }, size = 24): HTMLElement {
  const el = h("span", { class: `${P}av ${P}c${tone(a.id)}`, text: a.name.slice(0, 2) });
  el.style.setProperty("--size", `${size}px`);
  return el;
}

function hubWindow(root: HTMLElement, cx: FeatureCtx) {
  let agents: Agent[] = [];
  let tasks: Task[] = [];
  let feed: Msg[] = [];
  let current = KNOWN[0].id;
  let panelOpen = true;
  let off = false;
  let view: "agent" | "skills" | "folder" = "agent";
  let skills: Skill[] = [];
  let editing: Partial<Skill> | null = null;
  let attached: Attachment | null = null;
  const saved = new Set<number>();
  let asks: Ask[] = [];
  let folderAuto = false;
  let automation = new Set<string>();
  let confirming = "";
  let folder = "";
  let listing: Listing | null = null;
  let file: { path: string; name: string; text: string } | null = null;
  let query = "";

  const tabs = h("div", { class: `${P}tabs` });
  const stage = h("div", { class: `${P}stage` });
  const panel = h("aside", { class: `${P}panel` });
  const body = h("div", { class: `${P}body` }, stage, panel);
  const nameOf = (id: string) => agents.find((a) => a.id === id)?.name ?? id;
  const waiting = () => [...tasks.filter((t) => t.status === "proposed" || t.forward === "waiting"), ...asks.filter((a) => a.status === "pending")];
  const stateOf = (id: string): "waiting" | "working" | "idle" =>
    tasks.some((t) => t.status === "proposed" && t.to === id) || asks.some((a) => a.status === "pending" && a.agent === id) ? "waiting" : tasks.some((t) => t.status === "sent" && t.to === id) ? "working" : "idle";

  async function refresh() {
    const [a, t, f, k, q] = await Promise.all([
      cx.call<Agent[]>("participants"),
      cx.call<Task[]>("tasks"),
      cx.call<Msg[]>("history", { channel: "general" }),
      cx.call<Skill[]>("skills"),
      cx.call<Ask[]>("requests"),
    ]);
    asks = q ?? [];
    const conf = await cx.settings.get();
    folderAuto = conf.folderAuto === true;
    automation = new Set(Array.isArray(conf.automation) ? (conf.automation as string[]) : []);
    skills = k ?? [];
    folder = (await cx.call<{ folder: string }>("folder"))?.folder ?? "";
    off = a == null;
    agents = (a ?? []).filter((p) => p.kind !== "user");
    tasks = t ?? [];
    feed = f ?? [];
    for (const k of KNOWN) {
      if (!agents.some((p) => p.id === k.id) && !off) {
        agents.push({ id: k.id, name: k.name, kind: "web", caps: [] });
        void cx.call("join", { id: k.id, name: k.name, kind: "web", caps: [] });
      }
    }
    if (!agents.some((p) => p.id === current)) current = agents[0]?.id ?? "";
    draw();
  }

  // ── Tabs ────────────────────────────────────────────────────────────────
  function drawTabs() {
    const n = waiting().length;
    tabs.replaceChildren(
      ...agents.map((a) =>
        h("button", { class: `${P}tab${view === "agent" && a.id === current ? " on" : ""}`, onclick: () => ((current = a.id), (view = "agent"), draw()) },
          avatar(a, 18),
          h("span", { text: a.name }),
          h("i", { class: `${P}state ${P}${stateOf(a.id)}`, title: cx.t(stateOf(a.id) === "idle" ? "Idle" : stateOf(a.id) === "waiting" ? "Waiting for you" : "Working") })),
      ),
      h("button", { class: `${P}tab${view === "skills" ? " on" : ""}`, onclick: () => ((view = "skills"), draw()) },
        icon("file", 14), h("span", { text: cx.t("Skills") }), skills.length ? h("small", { class: `${P}count`, text: String(skills.length) }) : null),
      h("button", { class: `${P}tab${view === "folder" ? " on" : ""}`, onclick: () => ((view = "folder"), draw(), void browse(listing?.path ?? "")) },
        icon("shelf", 14), h("span", { text: cx.t("Folder") })),
      h("button", { class: `${P}tab ${P}more`, title: cx.t("Add an agent"), onclick: () => cx.island.notice({ kind: "agent", title: cx.t("More agents come with their adapters"), icon: "plus" }) },
        icon("plus", 14), h("span", { text: cx.t("Other") })),
      h("span", { class: `${P}fill` }),
      h("button", { class: `${P}hubbtn${panelOpen ? " on" : ""}`, title: cx.t("Hub"), onclick: () => ((panelOpen = !panelOpen), draw()) },
        icon("grid", 14), h("span", { text: cx.t("Hub") }), n ? h("b", { text: String(n) }) : null),
    );
  }

  // ── The stage: where the agent's own page appears ───────────────────────
  // ── Skills: what the user gathers, kept as text ─────────────────────────
  function saveSkill(draft: Partial<Skill> & { tagText?: string }) {
    const title = (draft.title ?? "").trim();
    const body = (draft.body ?? "").trim();
    if (!title || !body) return;
    void cx.call("skill_save", { id: draft.id, title, body, source: draft.source ?? "", tags: draft.tagText ?? "" }).then(() => {
      editing = null;
      void refresh();
    });
  }

  function skillForm(): HTMLElement {
    const draft: Partial<Skill> & { tagText?: string } = { ...editing, tagText: (editing?.tags ?? []).join(", ") };
    const title = field({ placeholder: cx.t("Name"), value: draft.title, onInput: (v) => (draft.title = v) });
    const source = field({ placeholder: cx.t("Where it came from (a link, optional)"), value: draft.source, onInput: (v) => (draft.source = v) });
    const tags = field({ placeholder: cx.t("Tags, separated by commas"), value: draft.tagText, onInput: (v) => (draft.tagText = v) });
    const text = field({ placeholder: cx.t("Paste what you gathered"), multiline: true, value: draft.body, onInput: (v) => (draft.body = v) });
    return card([
      title, source, tags, text,
      h("div", { class: `${P}actions` },
        btn(cx.t("Cancel"), "ghost", () => ((editing = null), draw())),
        btn(cx.t("Save"), "primary", () => saveSkill(draft))),
    ]);
  }

  function skillCard(k: Skill): HTMLElement {
    const on = attached?.key === `skill:${k.id}`;
    return card([
      h("div", { class: `${P}skill` },
        h("div", { class: `${P}skill-h` },
          h("b", { text: k.title }),
          h("span", { class: `${P}fill` }),
          btn(cx.t(on ? "Attached" : "Attach to hand-off"), on ? "secondary" : "ghost", () => ((attached = on ? null : { key: `skill:${k.id}`, title: k.title, body: k.body }), draw())),
          iconBtn("text", cx.t("Edit"), () => ((editing = k), draw())),
          iconBtn("trash", cx.t("Delete"), () => void cx.call("skill_delete", { id: k.id }).then(() => ((attached = attached?.key === `skill:${k.id}` ? null : attached), refresh())))),
        h("p", { class: `${P}skill-b`, text: k.body }),
        h("div", { class: `${P}caps ${P}left` },
          ...k.tags.map((t) => h("span", { class: `${P}chip`, text: t })),
          k.source ? h("button", { class: `${P}chip ${P}link`, title: k.source, onclick: () => void Bridge.openUrl(k.source), text: k.source.replace(/^https?:\/\//, "").slice(0, 40) }) : null)),
    ]);
  }

  function skillsView(): HTMLElement {
    const q = query.trim().toLowerCase();
    const shown = skills.filter((k) => !q || `${k.title} ${k.body} ${k.tags.join(" ")}`.toLowerCase().includes(q));
    const search = field({ icon: "search", placeholder: cx.t("Search skills"), value: query, onInput: (v) => ((query = v), drawStage()) });
    return h("div", { class: `${P}skills` },
      h("div", { class: `${P}skills-h` }, search, btn(cx.t("Add skill"), "primary", () => ((editing = {}), draw()))),
      editing ? skillForm() : null,
      ...(shown.length
        ? shown.map(skillCard)
        : [h("p", { class: `${P}none`, text: cx.t(skills.length ? "Nothing matches." : "Nothing here yet. Add what you find on the web: a prompt, a how-to, a snippet.") })]),
    );
  }

  // ── Folder: the one folder the user shares, read-only ──────────────────
  async function browse(path: string) {
    file = null;
    listing = folder ? await cx.call<Listing>("folder_list", { path }) : null;
    drawStage();
  }

  async function open(e: Entry) {
    if (e.dir) return void browse(e.path);
    const r = await cx.call<{ name: string; text: string }>("folder_read", { path: e.path });
    file = r ? { path: e.path, name: r.name, text: r.text } : { path: e.path, name: e.name, text: "" };
    drawStage();
  }

  const kb = (n: number) => (n < 1024 ? `${n} B` : `${Math.ceil(n / 1024)} KB`);

  function folderView(): HTMLElement {
    const head = h("div", { class: `${P}skills-h` },
      h("div", { class: `${P}where`, title: folder, text: folder || cx.t("No folder is shared.") }),
      ...(folder ? [btn(cx.t("Stop sharing"), "ghost", () => void cx.call("folder_clear").then(() => ((folder = ""), (listing = null), (file = null), drawStage())))] : []),
      btn(cx.t(folder ? "Change folder" : "Choose folder"), "primary", () => void cx.call<{ folder: string }>("folder_pick").then((r) => ((folder = r?.folder ?? folder), browse("")))));
    if (!folder) {
      return h("div", { class: `${P}skills` }, head, h("p", { class: `${P}none`, text: cx.t("Your agents cannot see your disk. Share one folder here, and you can attach a file from it to a hand-off. Nothing leaves until you send it.") }));
    }
    const auto = card(row(cx.t("Auto mode"), toggle(folderAuto, (on) => {
      folderAuto = on;
      void cx.settings.patch({ folderAuto: on });
    }), cx.t("Agents can read this folder without asking each time. Secret files stay blocked. Saving a file always asks. Turn it on only for pages you trust.")));
    const up = listing?.path ? btn(cx.t("Up"), "ghost", () => void browse(listing!.path.split("/").slice(0, -1).join("/"))) : null;
    const rows = (listing?.entries ?? []).map((e) =>
      h("button", { class: `${P}file${e.blocked ? " blocked" : ""}`, disabled: e.blocked, onclick: () => void open(e), title: e.blocked ? cx.t("Files that usually hold secrets are not shared.") : e.path },
        icon(e.dir ? "shelf" : "file", 14), h("span", { text: e.name }), h("small", { text: e.dir ? "" : kb(e.size) })));
    const preview = file
      ? card([
          h("div", { class: `${P}skill` },
            h("div", { class: `${P}skill-h` }, h("b", { text: file.name }), h("span", { class: `${P}fill` }),
              ...(file.text
                ? [btn(cx.t(attached?.key === `file:${file.path}` ? "Attached" : "Attach to hand-off"), "secondary", () => {
                    const f = file!;
                    attached = attached?.key === `file:${f.path}` ? null : { key: `file:${f.path}`, title: f.name, body: f.text };
                    draw();
                  })]
                : []),
              iconBtn("close", cx.t("Close"), () => ((file = null), drawStage()))),
            h("pre", { class: `${P}pre`, text: file.text ? file.text.slice(0, 1500) + (file.text.length > 1500 ? "\n…" : "") : cx.t("This file cannot be shared: it is not text, it is over 200 KB, or it usually holds secrets.") })),
        ])
      : null;
    return h("div", { class: `${P}skills` }, head, auto, up, preview, h("div", { class: `${P}files` }, ...rows),
      listing?.truncated ? h("p", { class: `${P}none`, text: cx.t("Only the first 500 entries are shown.") }) : null);
  }

  function drawStage() {
    if (view === "folder") {
      stage.replaceChildren(folderView());
      return;
    }
    if (view === "skills") {
      const keep = stage.querySelector(`.${P}skills`)?.scrollTop ?? 0;
      stage.replaceChildren(skillsView());
      const box = stage.querySelector(`.${P}skills`);
      if (box) box.scrollTop = keep;
      return;
    }
    const a = agents.find((p) => p.id === current);
    if (off || !a) {
      stage.replaceChildren(h("div", { class: `${P}empty` }, icon("sparkle", 28), h("b", { text: cx.t("The hub is off") }), h("p", { text: cx.t("Turn it on in Settings to connect your agents.") })));
      return;
    }
    stage.replaceChildren(
      h("div", { class: `${P}empty` },
        avatar(a, 56),
        h("b", { text: a.name }),
        h("p", { text: cx.t("Its own page opens here, signed in as you. Nothing goes through an API.") }),
        h("div", { class: `${P}caps` }, ...(a.caps.length ? a.caps.map((c) => h("span", { class: `${P}chip`, text: c })) : [h("span", { class: `${P}chip dim`, text: cx.t("No tools read yet") })])),
      ),
    );
  }

  // ── The hub panel ───────────────────────────────────────────────────────
  function proposal(t: Task): HTMLElement {
    return card([
      h("div", { class: `${P}ask` },
        h("div", { class: `${P}route` }, avatar({ id: t.from, name: nameOf(t.from) }, 20), icon("chevr", 12), avatar({ id: t.to, name: nameOf(t.to) }, 20),
          h("span", { text: cx.t("{from} needs {need}", { from: nameOf(t.from), need: t.need }) })),
        h("p", { class: `${P}text`, text: t.text }),
        h("small", { text: cx.t("{to} has it. It only gets this if you say so.", { to: nameOf(t.to) }) }),
        h("div", { class: `${P}actions` },
          btn(cx.t("Skip"), "ghost", () => void cx.call("skip", { task: t.id }).then(refresh)),
          btn(cx.t("Send to {to}", { to: nameOf(t.to) }), "primary", () => void cx.call("approve", { task: t.id }).then(refresh))),
      ),
    ], true);
  }

  function askCard(a: Ask): HTMLElement {
    return card([
      h("div", { class: `${P}ask` },
        h("div", { class: `${P}route` }, avatar({ id: a.agent, name: nameOf(a.agent) }, 20),
          h("span", { text: cx.t(a.kind === "read" ? "{agent} wants to read a file" : a.kind === "list" ? "{agent} wants to see a folder" : "{agent} wants to save a file", { agent: nameOf(a.agent) }) })),
        h("p", { class: `${P}text ${P}mono`, text: a.path || "/" }),
        ...(a.kind === "write"
          ? [h("pre", { class: `${P}pre`, text: (a.content ?? "").slice(0, 1500) + ((a.content ?? "").length > 1500 ? "\n…" : "") }),
             h("small", { text: cx.t(a.replaces ? "This replaces a file in your folder. A copy of the old one is kept." : "This creates a new file in your folder.") })]
          : [h("small", { text: cx.t("From the folder you share. It only gets it if you say so.") })]),
        h("div", { class: `${P}actions` },
          btn(cx.t("Deny"), "ghost", () => void cx.call("request_deny", { id: a.id }).then(refresh)),
          btn(cx.t(a.kind === "write" ? "Save file" : "Allow"), "primary", () => void cx.call("request_allow", { id: a.id }).then(refresh)))),
    ], "warn");
  }

  function taskRow(t: Task): HTMLElement {
    const tone = t.status === "done" ? "ok" : t.status === "skipped" ? "dim" : "work";
    return h("div", { class: `${P}task` },
      h("i", { class: `${P}dot ${P}${tone}` }),
      h("div", { class: `${P}task-t` },
        h("b", { text: `${nameOf(t.from)} → ${nameOf(t.to)}` }),
        h("small", { text: t.result ?? t.text })),
      t.status === "done" && t.result
        ? saved.has(t.id)
          ? h("span", { class: `${P}pill`, text: cx.t("Saved as skill") })
          : iconBtn("file", cx.t("Save as skill"), () =>
              void cx.call("skill_save", { title: `${nameOf(t.to)}: ${t.text.split("\n")[0].slice(0, 60)}`, body: t.result, source: "", tags: `${t.need}, ${t.to}` }).then((r) => {
                if (r) saved.add(t.id);
                void refresh();
              }))
        : null,
      h("span", { class: `${P}pill`, text: cx.t(t.status === "sent" ? "Working" : t.status === "done" ? "Done" : "Skipped") }));
  }

  function handoffForm(): HTMLElement {
    const need = field({ placeholder: cx.t("Needs, e.g. supabase") });
    const text = field({ placeholder: cx.t("What should it do?") });
    const go = () => {
      const n = (need.input as HTMLInputElement).value.trim();
      const x = (text.input as HTMLInputElement).value.trim();
      if (!n || !x || !current) return;
      const full = attached ? `${cx.t("Reference")} — ${attached.title}:\n${attached.body}\n\n${cx.t("Task")}: ${x}` : x;
      void cx.call("handoff", { channel: "general", from: current, need: n, text: full }).then((r) => {
        if (r == null) cx.island.notice({ kind: "agent", title: cx.t("No agent has {need}", { need: n }), icon: "alert", tone: "warn" });
        else {
          (need.input as HTMLInputElement).value = "";
          (text.input as HTMLInputElement).value = "";
        }
        void refresh();
      });
    };
    const skill = attached ? h("button", { class: `${P}chip ${P}link`, title: cx.t("Remove"), onclick: () => ((attached = null), drawPanel()), text: `${cx.t("Attached")}: ${attached.title} ×` }) : null;
    return card([need, text, skill, h("div", { class: `${P}actions` }, btn(cx.t("Propose hand-off"), "secondary", go))]);
  }

  // ── This agent: its tools, and whether the hub may drive its page ───────
  function setAutomation(id: string, on: boolean) {
    on ? automation.add(id) : automation.delete(id);
    confirming = "";
    capsTried.delete(id);
    void cx.settings.patch({ automation: [...automation] }).then(() => cx.call("agent_reset", { id })).then(() => {
      placed = "";
      draw();
    });
  }

  function agentCard(): HTMLElement | null {
    const a = agents.find((p) => p.id === current);
    const k = KNOWN.find((p) => p.id === current);
    if (!a || !k) return null;
    const setCaps = (caps: string[]) => void cx.call("set_caps", { id: a.id, caps }).then(refresh);
    const add = field({ placeholder: cx.t("Add a tool, e.g. supabase"), onEnter: (v) => {
      const name = v.trim().toLowerCase();
      if (name && !a.caps.includes(name)) setCaps([...a.caps, name]);
    } });
    const on = automation.has(a.id);
    return card([
      h("div", { class: `${P}route` }, avatar(a, 20), h("span", { text: cx.t("{agent}'s tools", { agent: a.name }) })),
      h("div", { class: `${P}caps ${P}left` },
        ...a.caps.map((c) => h("button", { class: `${P}chip ${P}link`, title: cx.t("Remove"), onclick: () => setCaps(a.caps.filter((x) => x !== c)), text: `${c} ×` })),
        a.caps.length ? null : h("span", { class: `${P}chip dim`, text: cx.t("None set") })),
      add,
      confirming === a.id
        ? h("div", { class: `${P}warn` },
            h("small", { text: cx.t("Automation drives this agent's page with a script, which its terms may forbid. Its account could be limited or closed. Use an account you can lose.") }),
            h("div", { class: `${P}actions` }, btn(cx.t("Cancel"), "ghost", () => ((confirming = ""), drawPanel())), btn(cx.t("Turn on"), "danger", () => setAutomation(a.id, true))))
        : row(cx.t("Automation"), toggle(on, (v) => (v ? ((confirming = a.id), drawPanel()) : setAutomation(a.id, false))),
            cx.t(on ? "On: the hub reads this page's tools and types approved tasks into it." : "Off: this is a plain browser tab. You copy tasks and paste replies yourself.")),
    ]);
  }

  // A finished reply goes back to the agent that asked, once the user says so.
  function backCard(t: Task): HTMLElement {
    const typed = automation.has(t.from);
    const text = `Reply from ${nameOf(t.to)} via the hub, to your task "${t.text.split("\n")[0]}":\n${t.result ?? ""}`;
    const go = () => {
      if (!typed) void navigator.clipboard?.writeText(text);
      void cx.call("forward_send", { task: t.id }).then((r) => {
        if (r == null) cx.island.notice({ kind: "agent", title: cx.t("It could not be sent. Try again, or copy it."), icon: "alert", tone: "warn" });
        void refresh();
      });
    };
    return card([
      h("div", { class: `${P}ask` },
        h("div", { class: `${P}route` }, avatar({ id: t.to, name: nameOf(t.to) }, 20), icon("chevr", 12), avatar({ id: t.from, name: nameOf(t.from) }, 20),
          h("span", { text: cx.t("{to} finished", { to: nameOf(t.to) }) })),
        h("p", { class: `${P}text`, text: (t.result ?? "").slice(0, 300) + ((t.result ?? "").length > 300 ? "…" : "") }),
        h("small", { text: cx.t(typed ? "Send this reply back to {from}? It is typed into its page." : "Copy this reply to take back to {from}?", { from: nameOf(t.from) }) }),
        h("div", { class: `${P}actions` },
          btn(cx.t("Skip"), "ghost", () => void cx.call("forward_skip", { task: t.id }).then(refresh)),
          btn(cx.t(typed ? "Send back to {from}" : "Copy reply", { from: nameOf(t.from) }), "primary", go))),
    ]);
  }

  function manualTask(t: Task): HTMLElement {
    const reply = field({ placeholder: cx.t("Paste the reply here"), multiline: true });
    return card([
      h("div", { class: `${P}ask` },
        h("div", { class: `${P}route` }, avatar({ id: t.to, name: nameOf(t.to) }, 20), h("span", { text: cx.t("Waiting for {to}", { to: nameOf(t.to) }) })),
        h("p", { class: `${P}text`, text: t.text }),
        h("div", { class: `${P}actions` }, btn(cx.t("Copy task"), "secondary", () => void navigator.clipboard?.writeText(`Task from ${nameOf(t.from)} via the hub:\n${t.text}`))),
        reply,
        h("div", { class: `${P}actions` },
          btn(cx.t("Done"), "primary", () => {
            const text = (reply.input as HTMLTextAreaElement).value.trim();
            if (text) void cx.call("finish", { task: t.id, result: text }).then(refresh);
          }))),
    ]);
  }

  function drawPanel() {
    panel.hidden = !panelOpen;
    const ask = tasks.filter((t) => t.status === "proposed");
    const wanted = asks.filter((a) => a.status === "pending");
    const seen = asks.filter((a) => a.status !== "pending").slice(-5).reverse();
    const rest = tasks.filter((t) => t.status !== "proposed" && !(t.status === "sent" && !automation.has(t.to))).reverse();
    const say = field({ placeholder: cx.t("Message the channel") });
    const sendMsg = () => {
      const input = say.input as HTMLInputElement;
      const text = input.value.trim();
      if (!text) return;
      input.value = "";
      void cx.call("post", { channel: "general", from: "you", text });
    };
    (say.input as HTMLInputElement).addEventListener("keydown", (e) => e.key === "Enter" && sendMsg());
    const manual = tasks.filter((t) => t.status === "sent" && !automation.has(t.to));
    const back = tasks.filter((t) => t.status === "done" && t.forward === "waiting");
    panel.replaceChildren(
      ...(view === "agent" ? [agentCard()].filter((c): c is HTMLElement => c != null) : []),
      h("div", { class: `${P}sec` }, h("span", { class: "section-title", text: cx.t("Needs your OK") })),
      ...manual.map(manualTask),
      ...back.map(backCard),
      ...wanted.map(askCard),
      ...(ask.length ? ask.map(proposal) : wanted.length || manual.length || back.length ? [] : [h("p", { class: `${P}none`, text: cx.t("Nothing is waiting. Hand-offs between agents show up here first.") })]),
      h("div", { class: `${P}sec` }, h("span", { class: "section-title", text: cx.t("Hand off") })),
      handoffForm(),
      ...(rest.length ? [h("div", { class: `${P}sec` }, h("span", { class: "section-title", text: cx.t("Tasks") })), h("div", { class: `${P}tasks` }, ...rest.map(taskRow))] : []),
      ...(seen.length ? [h("div", { class: `${P}sec` }, h("span", { class: "section-title", text: cx.t("Files shared") })),
        h("div", { class: `${P}tasks` }, ...seen.map((a) => h("div", { class: `${P}task` },
          h("i", { class: `${P}dot ${P}${a.status === "allowed" ? "ok" : "dim"}` }),
          h("div", { class: `${P}task-t` }, h("b", { text: `${nameOf(a.agent)} · ${a.kind}` }), h("small", { text: a.path || "/" })),
          h("span", { class: `${P}pill`, text: cx.t(a.status === "denied" ? "Denied" : a.auto ? "Auto" : "Allowed") }))))] : []),
      h("div", { class: `${P}sec` }, h("span", { class: "section-title", text: "# general" })),
      h("div", { class: `${P}feed` }, ...feed.slice(-30).map((m) => h("div", { class: `${P}msg` }, h("b", { text: m.from === "you" ? cx.t("You") : nameOf(m.from) }), h("span", { text: m.text })))),
      h("div", { class: `${P}say` }, say, iconBtn("send", cx.t("Send"), sendMsg, "primary")),
    );
    const f = panel.querySelector(`.${P}feed`);
    if (f) f.scrollTop = f.scrollHeight;
  }

  // The agent's own page is a native webview laid over the stage: say where the stage is.
  // Hidden while the hub is off or the agent has no known page (then the stage's message shows).
  let placed = "";
  const capsTried = new Set<string>();
  function place() {
    const k = KNOWN.find((a) => a.id === current);
    if (off || !k || view !== "agent") {
      if (placed) void cx.call("agent_hide");
      placed = "";
      return;
    }
    const r = stage.getBoundingClientRect();
    const rect = { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) };
    const key = `${k.id} ${rect.x} ${rect.y} ${rect.w} ${rect.h}`;
    if (key === placed || rect.w < 2 || rect.h < 2) return;
    placed = key;
    void cx.call("agent_show", { id: k.id, url: k.url, ...rect });
    // The page has to load before its tools can be read: once per agent, after a pause (agents with an adapter).
    if (k.adapter && automation.has(k.id) && !capsTried.has(k.id)) {
      capsTried.add(k.id);
      setTimeout(() => void cx.call("agent_caps", { id: k.id }).then((r) => !r && capsTried.delete(k.id)), 6000);
    }
  }
  new ResizeObserver(place).observe(stage);

  function draw() {
    drawTabs();
    drawStage();
    drawPanel();
    requestAnimationFrame(place);
  }

  root.append(
    h("div", { class: `win ${P}win` },
      h("div", { class: "win-bar" }, icon("sparkle", 14), h("span", { text: cx.t("Hub") }),
        h("button", { class: "close", title: cx.t("Close"), onclick: () => void cx.call("close"), text: "×" })),
      tabs,
      body),
  );
  for (const e of ["message", "proposal", "deliver", "done", "caps", "request"]) cx.on(e, () => void refresh());
  void refresh();
}

export default defineFeature({
  id: "hub",
  windows: { main: hubWindow },
});

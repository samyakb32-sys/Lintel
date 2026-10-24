// Hub — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// One window: a tab for every agent, and beside it the hub panel, where a
// hand-off waits for the user's OK before it is delivered to another agent.
// Rust owns the bus (crates/hub); this file only draws it. Nothing runs at import.

import "./style.css";
import { defineFeature, type FeatureCtx } from "../../core/feature";
import { h } from "../../views/dom";
import { icon } from "../../ui/icons";
import { btn, card, field, iconBtn } from "../../ui/components";

const P = "f-hub-";

interface Agent { id: string; name: string; kind: "user" | "cli" | "web"; caps: string[] }
interface Task { id: number; channel: string; from: string; to: string; need: string; text: string; status: "proposed" | "sent" | "done" | "skipped"; result: string | null }
interface Msg { id: number; channel: string; from: string; text: string; at: number }

// The agents offered before an adapter reports one. `tone` picks a token colour in style.css.
const KNOWN = [
  { id: "chatgpt", name: "ChatGPT", tone: 1, url: "https://chatgpt.com" },
  { id: "claude", name: "Claude", tone: 2, url: "https://claude.ai" },
  { id: "gemini", name: "Gemini", tone: 3, url: "https://gemini.google.com" },
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

  const tabs = h("div", { class: `${P}tabs` });
  const stage = h("div", { class: `${P}stage` });
  const panel = h("aside", { class: `${P}panel` });
  const body = h("div", { class: `${P}body` }, stage, panel);
  const nameOf = (id: string) => agents.find((a) => a.id === id)?.name ?? id;
  const waiting = () => tasks.filter((t) => t.status === "proposed");
  const stateOf = (id: string): "waiting" | "working" | "idle" =>
    tasks.some((t) => t.status === "proposed" && t.to === id) ? "waiting" : tasks.some((t) => t.status === "sent" && t.to === id) ? "working" : "idle";

  async function refresh() {
    const [a, t, f] = await Promise.all([
      cx.call<Agent[]>("participants"),
      cx.call<Task[]>("tasks"),
      cx.call<Msg[]>("history", { channel: "general" }),
    ]);
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
        h("button", { class: `${P}tab${a.id === current ? " on" : ""}`, onclick: () => ((current = a.id), draw()) },
          avatar(a, 18),
          h("span", { text: a.name }),
          h("i", { class: `${P}state ${P}${stateOf(a.id)}`, title: cx.t(stateOf(a.id) === "idle" ? "Idle" : stateOf(a.id) === "waiting" ? "Waiting for you" : "Working") })),
      ),
      h("button", { class: `${P}tab ${P}more`, title: cx.t("Add an agent"), onclick: () => cx.island.notice({ kind: "agent", title: cx.t("More agents come with their adapters"), icon: "plus" }) },
        icon("plus", 14), h("span", { text: cx.t("Other") })),
      h("span", { class: `${P}fill` }),
      h("button", { class: `${P}hubbtn${panelOpen ? " on" : ""}`, title: cx.t("Hub"), onclick: () => ((panelOpen = !panelOpen), draw()) },
        icon("grid", 14), h("span", { text: cx.t("Hub") }), n ? h("b", { text: String(n) }) : null),
    );
  }

  // ── The stage: where the agent's own page appears ───────────────────────
  function drawStage() {
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

  function taskRow(t: Task): HTMLElement {
    const tone = t.status === "done" ? "ok" : t.status === "skipped" ? "dim" : "work";
    return h("div", { class: `${P}task` },
      h("i", { class: `${P}dot ${P}${tone}` }),
      h("div", { class: `${P}task-t` },
        h("b", { text: `${nameOf(t.from)} → ${nameOf(t.to)}` }),
        h("small", { text: t.result ?? t.text })),
      h("span", { class: `${P}pill`, text: cx.t(t.status === "sent" ? "Working" : t.status === "done" ? "Done" : "Skipped") }));
  }

  function handoffForm(): HTMLElement {
    const need = field({ placeholder: cx.t("Needs, e.g. supabase") });
    const text = field({ placeholder: cx.t("What should it do?") });
    const go = () => {
      const n = (need.input as HTMLInputElement).value.trim();
      const x = (text.input as HTMLInputElement).value.trim();
      if (!n || !x || !current) return;
      void cx.call("handoff", { channel: "general", from: current, need: n, text: x }).then((r) => {
        if (r == null) cx.island.notice({ kind: "agent", title: cx.t("No agent has {need}", { need: n }), icon: "alert", tone: "warn" });
        else {
          (need.input as HTMLInputElement).value = "";
          (text.input as HTMLInputElement).value = "";
        }
        void refresh();
      });
    };
    return card([need, text, h("div", { class: `${P}actions` }, btn(cx.t("Propose hand-off"), "secondary", go))]);
  }

  function drawPanel() {
    panel.hidden = !panelOpen;
    const ask = waiting();
    const rest = tasks.filter((t) => t.status !== "proposed").reverse();
    const say = field({ placeholder: cx.t("Message the channel") });
    const sendMsg = () => {
      const input = say.input as HTMLInputElement;
      const text = input.value.trim();
      if (!text) return;
      input.value = "";
      void cx.call("post", { channel: "general", from: "you", text });
    };
    (say.input as HTMLInputElement).addEventListener("keydown", (e) => e.key === "Enter" && sendMsg());
    panel.replaceChildren(
      h("div", { class: `${P}sec` }, h("span", { class: "section-title", text: cx.t("Needs your OK") })),
      ...(ask.length ? ask.map(proposal) : [h("p", { class: `${P}none`, text: cx.t("Nothing is waiting. Hand-offs between agents show up here first.") })]),
      h("div", { class: `${P}sec` }, h("span", { class: "section-title", text: cx.t("Hand off") })),
      handoffForm(),
      ...(rest.length ? [h("div", { class: `${P}sec` }, h("span", { class: "section-title", text: cx.t("Tasks") })), h("div", { class: `${P}tasks` }, ...rest.map(taskRow))] : []),
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
    if (off || !k) {
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
    // The page has to load before its tools can be read: once per agent, after a pause (adapters: claude only).
    if (k.id === "claude" && !capsTried.has(k.id)) {
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
  for (const e of ["message", "proposal", "deliver", "done", "caps"]) cx.on(e, () => void refresh());
  void refresh();
}

export default defineFeature({
  id: "hub",
  windows: { main: hubWindow },
});

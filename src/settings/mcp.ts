// Settings → Coding agents → MCP servers: a short list of servers, each one
// connected to Claude Code or Cursor on its own click. Same steps as the hooks
// (agents.ts): the exact diff, then a click, and only then does mcp.rs back the
// file up and write it.

import { Bridge, type McpServer } from "../core/bridge";
import { clear, h } from "../views/dom";
import { renderDiff, statusDot } from "./parts";
import { t } from "../i18n/i18n";

/** In the current language (src/i18n); the settings window redraws on a change. */
const TEXT = {
  get title() { return t("MCP servers"); },
  get intro() {
    return t("An MCP server gives a coding agent more tools, such as GitHub or Vercel. Add one here, then connect it to the agent you want. You see the exact change first, and a copy of the old file is saved. Servers that need a secret key are not supported yet.");
  },
  get none() { return t("No servers yet."); },
  get add() { return t("Add a server"); },
  get name() { return t("Name"); },
  get line() { return t("Command or link"); },
  get save() { return t("Save server"); },
  get delete() { return t("Delete server"); },
  get install() { return t("Connect…"); },
  get uninstall() { return t("Disconnect…"); },
  get confirmInstall() { return t("Save a copy and apply"); },
  get confirmRemove() { return t("Save a copy and remove"); },
  get cancel() { return t("Cancel"); },
  get back() { return t("Back"); },
  previewInstall: (name: string) => t("This is exactly what changes for {name}. Nothing else is touched.", { name }),
  get previewRemove() { return t("This removes only what {app} added. Everything else stays."); },
  backup: (to: string) => (to ? t("Old file saved to: {path}", { path: to }) : t("There is no file yet, so nothing to copy.")),
  done: (backups: string, name: string) => {
    const saved = backups
      ? t("Done. The old file was saved as {paths}.", { paths: backups.split("\n").join(", ") })
      : t("Done.");
    return `${saved} ${t("Restart {name} to pick the server up.", { name })}`;
  },
  failed: (err: unknown) => t("Could not write: {error}", { error: errorText(err) }),
};

function errorText(err: unknown): string {
  return String(err).replace(/^Error:\s*/, "");
}

export function mcpSection(initial: McpServer[] | null): HTMLElement {
  const blocks = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  async function redraw() {
    draw((await Bridge.mcpList()) ?? []);
  }

  function draw(list: McpServer[]) {
    clear(blocks);
    if (list.length === 0) blocks.append(h("div", { class: "hint", text: TEXT.none }));
    for (const server of list) blocks.append(serverBlock(server, redraw));
  }

  draw(initial ?? []);
  return h(
    "section",
    {},
    h("h2", { class: "section-title" }, h("span", { text: TEXT.title })),
    h("div", { class: "hint", text: TEXT.intro }),
    blocks,
    addForm(redraw),
  );
}

function addForm(redraw: () => Promise<void>): HTMLElement {
  const name = h("input", { type: "text", placeholder: TEXT.name, spellcheck: "false", style: "width:140px" }) as HTMLInputElement;
  const line = h("input", {
    type: "text",
    placeholder: "npx -y @vercel/mcp   |   https://example.com/mcp",
    spellcheck: "false",
    style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  const save = h("button", { class: "btn secondary", text: TEXT.save }) as HTMLButtonElement;
  const feedback = h("div", {});
  save.addEventListener("click", async () => {
    save.disabled = true;
    clear(feedback);
    try {
      await Bridge.mcpSave(name.value, line.value);
      name.value = "";
      line.value = "";
      await redraw();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: errorText(err) }));
    }
    save.disabled = false;
  });
  return h(
    "div",
    { style: "display:flex;flex-direction:column;gap:8px;margin-top:14px" },
    h("div", { class: "hint", text: TEXT.add }),
    h("div", { class: "line" }, name, line, save),
    feedback,
  );
}

function serverBlock(server: McpServer, redraw: () => Promise<void>): HTMLElement {
  const remove = h("button", { class: "btn danger", text: TEXT.delete }) as HTMLButtonElement;
  const feedback = h("div", {});
  remove.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.mcpDelete(server.name);
      await redraw();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: errorText(err) }));
    }
  });
  const head = h(
    "div",
    { class: "line" },
    h("span", { style: "font-size:12.5px;min-width:132px", text: server.name }),
    h("span", { class: "hint", text: server.summary }),
    h("span", { class: "spacer" }),
    remove,
  );
  return h(
    "div",
    { style: "display:flex;flex-direction:column;gap:8px" },
    head,
    ...server.agents.map((agent) => agentRow(server, agent, redraw)),
    feedback,
  );
}

function agentRow(server: McpServer, agent: McpServer["agents"][number], redraw: () => Promise<void>): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:10px" });
  const button = h("button", {
    class: agent.connected ? "btn danger" : "btn primary",
    text: agent.connected ? TEXT.uninstall : TEXT.install,
    onclick: () => void showPreview(!agent.connected),
  });
  const head = h(
    "div",
    { class: "line" },
    h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px;padding-left:12px" },
      statusDot(agent.connected),
      h("span", { style: "font-size:12.5px", text: agent.name }),
    ),
    h("span", { class: "spacer" }),
    button,
  );

  async function showPreview(install: boolean) {
    let plan;
    try {
      plan = await Bridge.mcpPreview(server.name, agent.id, install);
    } catch (err) {
      // An unreadable file, or an entry that is not ours, stops here.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: errorText(err) }),
        h("div", { class: "line" }, h("button", { class: "btn secondary", text: TEXT.back, onclick: () => clear(body) })),
      );
      return;
    }
    clear(body);
    body.append(
      h("div", { class: "hint", text: install ? TEXT.previewInstall(agent.name) : TEXT.previewRemove }),
      renderDiff(plan.diff),
      h("div", { class: "line" }, h("span", { class: "path", text: TEXT.backup(plan.backup) })),
    );
    const confirm = h("button", {
      class: install ? "btn primary" : "btn danger",
      text: install ? TEXT.confirmInstall : TEXT.confirmRemove,
    });
    confirm.addEventListener("click", async () => {
      confirm.disabled = true;
      try {
        const backups = await Bridge.mcpApply(server.name, agent.id, install, plan.fingerprint);
        clear(body);
        body.append(h("div", { class: "notice ok", text: TEXT.done(backups, agent.name) }));
        window.setTimeout(() => void redraw(), 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: TEXT.failed(err) }));
      }
    });
    body.append(h("div", { class: "line" }, confirm, h("button", { class: "btn secondary", text: TEXT.cancel, onclick: () => clear(body) })));
  }

  return h("div", { style: "display:flex;flex-direction:column;gap:8px" }, head, body);
}

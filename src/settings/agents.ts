// Settings → Agents: one section for every agent other than Claude Code, all
// going through the same steps — preview the exact diff, click to confirm, and
// only then does agents.rs back the file up and write it.

import { Bridge, type AgentHookStatus } from "../core/bridge";
import { clear, h } from "../views/dom";
import { renderDiff, statusDot } from "./parts";
import { t } from "../i18n/i18n";

/** In the current language (src/i18n); the settings window redraws on a change. */
const TEXT = {
  get title() { return t("Other coding agents"); },
  get intro() {
    return t("Connect another coding agent to see its sessions on the island. Before anything is changed you see the exact change and where a copy of the old file is saved. Disconnecting removes only what {app} added.");
  },
  get none() { return t("{app} could not list the agents."); },
  get approvals() { return t("Shows sessions. You can Allow or Deny from the island."); },
  get displayOnly() { return t("Shows sessions only. You approve in the agent itself."); },
  get install() { return t("Connect…"); },
  get reinstall() { return t("Reconnect…"); },
  get uninstall() { return t("Disconnect…"); },
  get relayMissing() { return t("The helper program is missing. Restart {app}."); },
  previewInstall: (name: string) => t("This is exactly what changes for {name}. Nothing else is touched.", { name }),
  get previewRemove() { return t("This removes only what {app} added. Everything else stays."); },
  backup: (to: string) => (to ? t("Old file saved to: {path}", { path: to }) : t("There is no file yet, so nothing to copy.")),
  get confirmInstall() { return t("Save a copy and apply"); },
  get confirmRemove() { return t("Save a copy and remove"); },
  get cancel() { return t("Cancel"); },
  get back() { return t("Back"); },
  done: (backups: string, note: string) => {
    const saved = backups
      ? t("Done. The old file was saved as {paths}.", { paths: backups.split("\n").join(", ") })
      : t("Done.");
    return `${saved} ${note}`;
  },
  failed: (err: unknown) => t("Could not write: {error}", { error: errorText(err) }),
};

function errorText(err: unknown): string {
  return String(err).replace(/^Error:\s*/, "");
}

export function agentsSection(list: AgentHookStatus[] | null): HTMLElement {
  const blocks = h("div", { style: "display:flex;flex-direction:column;gap:14px" });
  if (!list || list.length === 0) {
    blocks.append(h("div", { class: "hint", text: TEXT.none }));
  }
  for (const status of list ?? []) blocks.append(agentBlock(status));
  return h(
    "section",
    {},
    h("h2", { class: "section-title" }, h("span", { text: TEXT.title })),
    h("div", { class: "hint", text: TEXT.intro }),
    blocks,
  );
}

function agentBlock(initial: AgentHookStatus): HTMLElement {
  let status = initial;
  const head = h("div", { class: "line" });
  const body = h("div", { style: "display:flex;flex-direction:column;gap:10px" });
  const block = h("div", { style: "display:flex;flex-direction:column;gap:8px" }, head, body);

  async function refresh() {
    const fresh = (await Bridge.agentHooksList())?.find((a) => a.id === status.id);
    if (fresh) status = fresh;
    draw();
  }

  function draw() {
    clear(head);
    clear(body);
    const install = h("button", {
      class: "btn primary",
      text: status.installed ? TEXT.reinstall : TEXT.install,
      onclick: () => void showPreview(true),
    });
    // A hook pointing at a relay that isn't there would break the agent's hooks.
    if (!status.hookReady) {
      install.disabled = true;
      install.title = TEXT.relayMissing;
    }
    head.append(
      h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px" },
        statusDot(status.installed),
        h("span", { style: "font-size:12.5px", text: status.name }),
      ),
      h("span", { class: "hint", text: status.approvals ? TEXT.approvals : TEXT.displayOnly }),
      h("span", { class: "spacer" }),
      install,
    );
    if (status.installed) {
      head.append(h("button", {
        class: "btn danger",
        text: TEXT.uninstall,
        onclick: () => void showPreview(false),
      }));
    }
    // The agent's config file is a detail; the connect preview shows what changes in it.
    head.firstElementChild?.setAttribute("title", status.path);
  }

  async function showPreview(install: boolean) {
    let plan;
    try {
      plan = await Bridge.agentHooksPreview(status.id, install);
    } catch (err) {
      // An unreadable or unexpected config stops here rather than being
      // treated as empty and written over.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: errorText(err) }),
        h("div", { class: "line" }, h("button", { class: "btn secondary", text: TEXT.back, onclick: draw })),
      );
      return;
    }
    clear(body);
    body.append(
      h("div", { class: "hint", text: install ? TEXT.previewInstall(status.name) : TEXT.previewRemove }),
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
        const backups = await Bridge.agentHooksApply(status.id, install, plan.fingerprint);
        clear(body);
        body.append(h("div", { class: "notice ok", text: TEXT.done(backups, install ? status.note : "") }));
        window.setTimeout(() => void refresh(), 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: TEXT.failed(err) }));
      }
    });
    body.append(h("div", { class: "line" }, confirm, h("button", { class: "btn secondary", text: TEXT.cancel, onclick: draw })));
  }

  draw();
  return block;
}

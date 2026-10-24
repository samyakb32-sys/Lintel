// Hub adapter for claude.ai (injected into the agent's own webview, main frame only).
//
// It cannot call the app: the page has no IPC. The hub (crates/hub/src/webviews.rs)
// calls these functions with `eval` and reads what they return, one task at a time.
// Every function is synchronous and returns plain JSON; nothing here runs on a timer
// of its own, observes the page or sends anything by itself.
//
// Checked against a copy of claude.ai's composer, "+" menu and Connectors submenu (input, send, tools button,
// connector switches). STILL UNVERIFIED: the stop button and the reply elements.
// The selectors are guesses at claude.ai's markup, which changes without
// notice. They live in SEL, first match wins, and `state()` reports what it could not
// find in `missing`, so a broken adapter shows up as "not ready" and the hub disables it.
(() => {
  if (window.__hubAdapter) return;

  const SEL = {
    input: ['[data-testid="chat-input"]', 'div[contenteditable="true"][role="textbox"]', 'div.ProseMirror[contenteditable="true"]'],
    send: ['button[data-testid="chat-input-send"]', 'button[aria-label="Send message"]', 'button[aria-label="Send Message"]'],
    stop: ['button[aria-label="Stop response"]', 'button[aria-label="Stop Response"]'],
    reply: ['[data-testid="assistant-message"]', 'div.font-claude-message', 'div.font-claude-response'],
    tools: ['button[data-testid="chat-input-attach"]', 'button[aria-label="Add files, connectors, and more"]'],
    toolItem: ['[role="menuitemcheckbox"]', '[role="menuitem"]'],
  };

  const first = (list) => {
    for (const s of list) {
      const el = document.querySelector(s);
      if (el) return el;
    }
    return null;
  };
  const all = (list) => {
    for (const s of list) {
      const els = [...document.querySelectorAll(s)];
      if (els.length) return els;
    }
    return [];
  };
  // Every match of every selector: a menu mixes entry kinds, and the first kind alone would hide the rest.
  const union = (list) => [...new Set(list.flatMap((s) => [...document.querySelectorAll(s)]))];
  // The lines of an element's text, without the empty one an icon leaves in front of a menu entry's name.
  const lines = (el) => text(el).split("\n").map((l) => l.trim()).filter(Boolean);
  const text = (el) => (el ? (el.innerText || el.textContent || "").trim() : "");

  window.__hubAdapter = {
    /** Is the page usable, is a reply being written, and what did the agent say last. */
    state() {
      const missing = [];
      if (!first(SEL.input)) missing.push("input");
      const replies = all(SEL.reply);
      return {
        ready: missing.length === 0,
        missing,
        busy: !!first(SEL.stop),
        count: replies.length,
        last: text(replies[replies.length - 1]),
      };
    },

    /** Types the text into the composer and presses send a moment later. Called only after the user said yes in the hub. */
    send(message) {
      const input = first(SEL.input);
      if (!input) return { ok: false, error: "the message box was not found" };
      input.focus();
      document.execCommand("selectAll", false);
      document.execCommand("insertText", false, String(message));
      setTimeout(() => {
        const button = first(SEL.send);
        if (button && !button.disabled) button.click();
      }, 250);
      return { ok: true };
    },

    /** Step 1 of reading the connected tools: opens the composer's tools menu. */
    toolsOpen() {
      const button = first(SEL.tools);
      if (!button) return { ok: false };
      button.click();
      return { ok: true };
    },

    /**
     * Step 2, once the menu has drawn. The "+" menu lists Research and Memory itself, and the connectors in a
     * submenu: the first read opens that submenu and says `more`; the read after it collects the entries and
     * closes the menu. Entries of the top level are not tools of a task, except Research.
     */
    toolsRead() {
      const TOP = ["upload or add files", "skills", "connectors", "design system", "plugins", "devices", "research", "memory"];
      const found = new Set();
      const connectors = document.querySelector('[data-testid="add-menu-connectors"]');
      const submenuOpen = !!connectors && connectors.getAttribute("aria-expanded") === "true";
      for (const el of union(SEL.toolItem)) {
        const name = (lines(el)[0] || "").toLowerCase();
        if (!name) continue;
        if (name === "research") found.add("research"), found.add("browse");
        // The submenu's connectors are switches ("Canva", "Vercel", ... aria-checked); its plain entries
        // ("Add connector", "Manage connectors", "Add from …", "Tool access") are not tools.
        else if (el.getAttribute("role") === "menuitemcheckbox" && el.getAttribute("aria-checked") === "true" && !TOP.includes(name)) found.add(name);
      }
      if (connectors && !submenuOpen) {
        connectors.click();
        return { ok: true, more: true, tools: [...found] };
      }
      const button = first(SEL.tools);
      if (button && button.getAttribute("aria-expanded") === "true") button.click();
      return { ok: true, more: false, tools: [...found] };
    },
  };
})();

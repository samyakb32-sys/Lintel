// Hub adapter for claude.ai (injected into the agent's own webview, main frame only).
//
// It cannot call the app: the page has no IPC. The hub (crates/hub/src/webviews.rs)
// calls these functions with `eval` and reads what they return, one task at a time.
// Every function is synchronous and returns plain JSON; nothing here runs on a timer
// of its own, observes the page or sends anything by itself.
//
// UNVERIFIED: the selectors are guesses at claude.ai's markup, which changes without
// notice. They live in SEL, first match wins, and `state()` reports what it could not
// find in `missing`, so a broken adapter shows up as "not ready" and the hub disables it.
(() => {
  if (window.__hubAdapter) return;

  const SEL = {
    input: ['div[contenteditable="true"][role="textbox"]', 'div.ProseMirror[contenteditable="true"]', 'fieldset div[contenteditable="true"]'],
    send: ['button[aria-label="Send message"]', 'button[aria-label="Send Message"]', 'button[data-testid="send-button"]'],
    stop: ['button[aria-label="Stop response"]', 'button[aria-label="Stop Response"]'],
    reply: ['[data-testid="assistant-message"]', 'div.font-claude-message', 'div.font-claude-response'],
    tools: ['button[data-testid="input-menu-tools"]', 'button[aria-label="Search and tools"]', 'button[aria-label="Add files, connectors, and more"]'],
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

    /** Step 2, once the menu has drawn: its entries that are switched on, then the menu is closed. */
    toolsRead() {
      const on = all(SEL.toolItem)
        .filter((el) => el.getAttribute("aria-checked") === "true")
        .map((el) => text(el).split("\n")[0].toLowerCase())
        .filter(Boolean);
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      return { ok: true, tools: [...new Set(on)] };
    },
  };
})();

// Hub adapter for gemini.google.com (injected into the agent's own webview, main frame only).
// Same contract as claude.js: the hub calls these functions with `eval` and reads what
// they return, one task at a time; every function is synchronous and plain JSON, and
// nothing here runs on its own.
//
// UNVERIFIED: the selectors are guesses at Gemini's markup, which changes without
// notice. They live in SEL (first match wins); `state()` lists what it could not find in
// `missing`, so a broken adapter shows up as "not ready" and the hub says why.
(() => {
  if (window.__hubAdapter) return;

  const SEL = {
    input: ['rich-textarea div.ql-editor[contenteditable="true"]', 'div.ql-editor[contenteditable="true"]', 'div[contenteditable="true"][role="textbox"]'],
    send: ['button[aria-label="Send message"]', "button.send-button", 'button[mattooltip="Send message"]'],
    stop: ['button[aria-label="Stop response"]', "button.stop", 'button[aria-label="Stop generating"]'],
    reply: ["model-response message-content", "model-response .model-response-text", "message-content"],
    tools: ['button[aria-label="Tools"]', "toolbox-drawer button", 'button[aria-label="Open tools"]'],
    toolItem: ['[role="menuitemcheckbox"]', '[role="menuitem"]', "button[mat-list-item]", "toolbox-drawer-item button"],
  };

  // What a tool means for a hand-off: the words a task asks for.
  const MEANS = [
    [/deep research/, ["research", "browse"]],
    [/google search|web search|^search/, ["browse"]],
    [/canvas/, ["canvas"]],
    [/image/, ["image"]],
  ];

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
        if (button && !button.disabled && button.getAttribute("aria-disabled") !== "true") button.click();
      }, 300);
      return { ok: true };
    },

    /** Step 1 of reading the tools: opens the composer's tools drawer. */
    toolsOpen() {
      const button = first(SEL.tools);
      if (!button) return { ok: false };
      button.click();
      return { ok: true };
    },

    /** Step 2, once the drawer has drawn: what its entries mean for a task, then it is closed. */
    toolsRead() {
      const found = new Set();
      for (const el of union(SEL.toolItem)) {
        const name = text(el).split("\n")[0].toLowerCase();
        if (!name) continue;
        const means = MEANS.find(([re]) => re.test(name));
        if (means) means[1].forEach((w) => found.add(w));
        else if (el.getAttribute("aria-checked") === "true") found.add(name);
      }
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      return { ok: true, tools: [...found] };
    },
  };
})();

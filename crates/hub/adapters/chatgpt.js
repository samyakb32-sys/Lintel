// Hub adapter for chatgpt.com (injected into the agent's own webview, main frame only).
// Same contract as claude.js: the hub calls these functions with `eval` and reads what
// they return, one task at a time; every function is synchronous and plain JSON, and
// nothing here runs on its own.
//
// Checked against a real page so far: the message box and the "+" button. STILL UNVERIFIED: the send and stop
// buttons, the replies and the menu's entries. The rest are guesses at chatgpt.com's markup, which changes without
// notice. They live in SEL (first match wins); `state()` lists what it could not find in
// `missing`, so a broken adapter shows up as "not ready" and the hub says why.
(() => {
  if (window.__hubAdapter) return;

  const SEL = {
    input: ['div.ProseMirror[contenteditable="true"][role="textbox"]', 'div[contenteditable="true"][aria-label="Ask ChatGPT"]', "#prompt-textarea", 'div[contenteditable="true"][role="textbox"]'],
    send: ['button[data-testid="send-button"]', 'button[aria-label="Send prompt"]', 'button[aria-label="Send message"]'],
    stop: ['button[data-testid="stop-button"]', 'button[aria-label="Stop streaming"]', 'button[aria-label="Stop generating"]'],
    reply: ['[data-message-author-role="assistant"]', 'article [data-message-author-role="assistant"]'],
    tools: ['button[data-composer-navigation-target="add-context"]', 'button[aria-label="Add files and more"]', 'button[data-testid="composer-plus-btn"]'],
    toolItem: ['[role="menuitemcheckbox"]', '[role="menuitemradio"]', '[role="menuitem"]'],
  };

  // What a menu entry means for a hand-off: the words a task asks for.
  const MEANS = [
    [/web search|search the web|^search/, ["browse"]],
    [/deep research/, ["research", "browse"]],
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
      if (input.tagName === "TEXTAREA") {
        input.value = String(message);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      } else {
        document.execCommand("selectAll", false);
        document.execCommand("insertText", false, String(message));
      }
      setTimeout(() => {
        const button = first(SEL.send);
        if (button && !button.disabled) button.click();
      }, 250);
      return { ok: true };
    },

    /** Step 1 of reading the tools: opens the composer's "+" menu. */
    toolsOpen() {
      const button = first(SEL.tools);
      if (!button) return { ok: false };
      button.click();
      return { ok: true };
    },

    /** Step 2, once the menu has drawn: what its entries mean for a task, then the menu is closed. */
    toolsRead() {
      const found = new Set();
      for (const el of union(SEL.toolItem)) {
        const name = (lines(el)[0] || "").toLowerCase();
        if (!name) continue;
        // A connector is listed under its own name; a built-in tool by what it lets a task ask for.
        const means = MEANS.find(([re]) => re.test(name));
        if (means) means[1].forEach((w) => found.add(w));
        else if (el.getAttribute("aria-checked") === "true") found.add(name);
      }
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      return { ok: true, tools: [...found] };
    },
  };
})();

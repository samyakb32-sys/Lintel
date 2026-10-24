// Hub adapter for grok.com (injected into the agent's own webview, main frame only).
// Same contract as claude.js: the hub calls these functions with `eval` and reads what
// they return, one task at a time; every function is synchronous and plain JSON, and
// nothing here runs on its own.
//
// UNVERIFIED: the selectors are guesses at grok.com's markup, which changes without
// notice. They live in SEL (first match wins); `state()` lists what it could not find in
// `missing`, so a broken adapter shows up as "not ready" and the hub says why.
(() => {
  if (window.__hubAdapter) return;

  const SEL = {
    input: ['textarea[aria-label*="Grok" i]', "form textarea", 'div[contenteditable="true"][role="textbox"]', "textarea"],
    send: ['button[type="submit"]', 'button[aria-label="Submit"]', 'button[aria-label="Send"]'],
    stop: ['button[aria-label="Stop model response"]', 'button[aria-label="Stop"]', 'button[aria-label="Stop generating"]'],
    reply: ["div.message-bubble", '[id^="response-"]', "div.response-content-markdown"],
    // Grok has no tools menu: its modes are buttons in the composer itself.
    modes: ["form button", "form [role=button]"],
  };

  // What a composer mode means for a hand-off: the words a task asks for.
  const MEANS = [
    [/deepsearch|deep search/, ["research", "browse"]],
    [/think/, ["reasoning"]],
    [/search/, ["browse"]],
    [/image|imagine/, ["image"]],
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
        // React ignores a plain `.value =`: go through the native setter, then tell it.
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set.call(input, String(message));
        input.dispatchEvent(new Event("input", { bubbles: true }));
      } else {
        document.execCommand("selectAll", false);
        document.execCommand("insertText", false, String(message));
      }
      setTimeout(() => {
        const button = first(SEL.send);
        if (button && !button.disabled) button.click();
      }, 300);
      return { ok: true };
    },

    /** Nothing to open: the modes are already in the composer. */
    toolsOpen() {
      return { ok: true };
    },

    /** What the composer's mode buttons mean for a task. */
    toolsRead() {
      const found = new Set();
      for (const el of all(SEL.modes)) {
        const name = text(el).toLowerCase();
        const means = name && MEANS.find(([re]) => re.test(name));
        if (means) means[1].forEach((w) => found.add(w));
      }
      return { ok: true, tools: [...found] };
    },
  };
})();

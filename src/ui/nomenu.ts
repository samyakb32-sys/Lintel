// No browser menu (Back, Reload, Print, Save as…) on the product's own pages: the web view's menu is
// not part of the design, and "Print" in it hid the island. A text field keeps the menu, for Cut, Copy and
// Paste; a feature that draws a menu of its own handles `contextmenu` itself and is not touched.
// Ctrl+P (print) does nothing either. Call once per page, from its entry point.

const EDITABLE = "input, textarea, select, [contenteditable]:not([contenteditable='false'])";

/** `true` for a right-click that may keep the web view's own menu. */
export function keepsMenu(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(EDITABLE) !== null;
}

export function installNoBrowserMenu() {
  document.addEventListener("contextmenu", (e) => {
    if (!keepsMenu(e.target)) e.preventDefault();
  });
  window.addEventListener("keydown", (e) => {
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "p") e.preventDefault();
  });
}

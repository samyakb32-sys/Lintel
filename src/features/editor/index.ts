// Editor — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// Nothing runs at import: declare surfaces here, begin work in `start`.
//
// Three window pages and no section. `editor` is the window a capture is
// edited in (editor.ts); `draw` is the see-through window for drawing on the
// live screen and `scroll` the selector and bar of the scrolling capture
// (overlay.ts). All three draw with the marks of draw.ts.
//
// Another feature opens the editor on a picture with one call:
//   Bridge.featureCall("editor", "open", { path })      a picture file
//   Bridge.featureCall("editor", "open", { capture })   a kept capture, by its file's name

import "./style.css";
import { defineFeature, type FeatureCtx } from "../../core/feature";
import { btn, card, row } from "../../ui/components";
import { h } from "../../views/dom";
import { ICONS } from "./draw";
import { mountEditor } from "./editor";
import { mountDraw, mountScroll } from "./overlay";

function settingsBody(cx: FeatureCtx): HTMLElement {
  const t = cx.t;
  return h(
    "div",
    { class: "f-editor-settings" },
    card([
      row(
        t("Draw on the screen"),
        btn(t("Start"), "secondary", () => void cx.call("draw")),
        t("A pen, a highlighter, arrows, shapes and text over everything on the display under the pointer. Esc leaves."),
      ),
      row(
        t("Capture a scrolling page"),
        btn(t("Start"), "secondary", () => void cx.call("scroll")),
        t("Drag over the part of a page that scrolls, scroll it to its end, then press Done. The long picture opens in the editor."),
      ),
    ]),
    h("p", {
      class: "f-editor-about",
      text: t("Edit on a capture opens it in the editor, where you can add arrows, shapes, text, highlight, blur, crop, a background and a watermark. Both tools above can have a shortcut under Shortcuts. Pictures are edited on this computer."),
    }),
  );
}

export default defineFeature({
  id: "editor",
  settings: settingsBody,
  windows: { editor: mountEditor, draw: mountDraw, scroll: mountScroll },
  commands: (_query, cx) => [
    { id: "editor:draw", title: cx.t("Draw on the screen"), icon: ICONS.pen, keywords: ["annotate", "pen", "highlighter", "arrow"], run: async () => void (await cx.call("draw")) },
    { id: "editor:scroll", title: cx.t("Capture a scrolling page"), icon: ICONS.scroll, keywords: ["screenshot", "long", "scrolling"], run: async () => void (await cx.call("scroll")) },
  ],
  start: (cx) =>
    // A long capture that is not all there says why; a complete one just opens.
    cx.on<{ outcome: string }>("scrolled", ({ outcome }) => {
      const t = cx.t;
      if (outcome === "complete") return;
      const detail =
        outcome === "partial" ? t("The page changed. The complete part is kept.")
        : outcome === "limited" ? t("Stopped at the largest size.")
        : t("The page could not be captured.");
      cx.island.notice({ kind: "capture", title: t("Scrolling capture"), detail, icon: ICONS.scroll, tone: "warn" });
    }),
});

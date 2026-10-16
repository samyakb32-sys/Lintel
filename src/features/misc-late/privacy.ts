// What the features that are running use, kind by kind. Built from the manifests
// (`uses`) and which features are on: Windows has no per-app grant for hooks,
// capture or the clipboard, so this page is the disclosure. Read when the page
// opens, never polled.

import { featureInfos } from "../../core/registry";
import { N_, t, tn } from "../../i18n/i18n";
import { btn } from "../../ui/components";
import { icon } from "../../ui/icons";
import { h } from "../../views/dom";
import { USE_TEXT } from "./presets";

type Use = keyof typeof USE_TEXT;

/** Why a window or key may still not answer: Windows' own rule. */
const ELEVATED = N_("Windows does not let an app without administrator rights control windows that have them. Those windows are left alone.");

/** The Windows settings page behind a use, when there is one. */
const SETTINGS_PAGE: Partial<Record<Use, { page: string; label: string }>> = {
  camera: { page: "privacy-webcam", label: N_("Camera privacy settings") },
  microphone: { page: "privacy-microphone", label: N_("Microphone privacy settings") },
};

export function privacySection(openSettings: (page: string) => void): HTMLElement {
  const infos = featureInfos();
  const rows = (Object.keys(USE_TEXT) as Use[]).map((use) => {
    const meta = USE_TEXT[use];
    const users = infos.filter((f) => f.running && f.uses.includes(use));
    const idle = infos.filter((f) => !f.running && f.uses.includes(use));
    const link = SETTINGS_PAGE[use];
    return h("div", { class: `f-misc-late-priv${users.length ? " active" : ""}` },
      h("span", { class: "f-misc-late-pico" }, icon(meta.icon, 16)),
      h("div", { class: "f-misc-late-privtext" },
        h("b", { text: t(meta.name) }),
        h("span", { class: "hint", text: t(meta.about) }),
        users.length
          ? h("span", { class: "f-misc-late-used", text: tn("Used by {names}", "Used by {names}", users.length, { names: users.map((f) => t(f.name)).join(", "), count: users.length }) })
          : h("span", { class: "hint", text: t("Nothing that is on uses this.") }),
        idle.length ? h("span", { class: "hint", text: t("Off: {names}", { names: idle.map((f) => t(f.name)).join(", ") }) }) : null,
      ),
      link ? btn(t(link.label), "ghost", () => openSettings(link.page)) : null,
    );
  });
  return h("section", {},
    h("h2", { class: "section-title", text: t("What is running uses") }),
    h("p", { class: "hint", text: t("Only features that are on and running are counted. Nothing is sent anywhere unless a feature says so and you set it up.") }),
    ...rows,
    h("p", { class: "hint", text: t(ELEVATED) }),
  );
}

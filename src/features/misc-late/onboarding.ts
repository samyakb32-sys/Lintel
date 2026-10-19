// The first-run page. The Settings window shows it before anything else until it
// has been dismissed once (any of its three buttons); after that it is gone for
// good. Nothing is switched on until the Apply button is pressed and confirmed.

import { BRAND } from "../../core/brand";
import { Bridge } from "../../core/bridge";
import { featureInfos } from "../../core/registry";
import { t } from "../../i18n/i18n";
import { btn } from "../../ui/components";
import { h } from "../../views/dom";
import { applyPreset, presetChooser } from "./presets";

const ID = "misc-late";

/** Has the page been dismissed? A feature that is turned off never asks. */
export async function needsOnboarding(): Promise<boolean> {
  const mine = featureInfos().find((f) => f.id === ID);
  if (!mine?.on) return false;
  const s = await Bridge.featureSettings(ID);
  return s?.onboarded !== true;
}

/**
 * Draws the page into `root` and resolves once it has been dismissed. Resolves
 * at once when it is not the first run. `go` is where the Settings window
 * should land next (the Agents page, so the hooks can be reviewed).
 */
export async function showOnboarding(root: HTMLElement): Promise<void> {
  if (!(await needsOnboarding())) return;
  const infos = featureInfos();
  await new Promise<void>((resolve) => {
    const done = async (next?: string) => {
      await Bridge.featurePatchSettings(ID, { onboarded: true });
      if (next) location.hash = `#/${next}`;
      resolve();
    };
    const agents = infos.find((f) => f.id === "agents");
    root.replaceChildren(
      h("div", { class: "win-body f-misc-late-first" },
        h("div", { class: "page" },
          h("h1", { text: t("Welcome to {app}", { app: BRAND.name }) }),
          h("p", { class: "hint", text: t("It stays out of the way at the top of your screen and comes out when something needs you. Pick a bundle to start with, or keep things as they are. You can change every switch later in Settings.") }),
          h("section", {},
            h("h2", { class: "section-title", text: t("Start with a bundle") }),
            presetChooser(infos, async (p) => {
              await applyPreset(p, featureInfos());
              await done(agents ? "f-agents" : undefined);
            }, t("Apply this bundle")),
          ),
          h("p", { class: "hint", text: t("Nothing is turned on until you press Apply. Claude Code and other agents are connected from the Agents page, after you see exactly what will change in their settings.") }),
          h("div", { class: "f-misc-late-actions" },
            btn(t("Keep my setup"), "secondary", () => void done()),
            btn(t("Skip for now"), "ghost", () => void done()),
          ),
        ),
      ),
    );
  });
}

// Hub — the page side of the feature (docs/ARCHITECTURE.md §2.3).
// Nothing runs at import: declare surfaces here, begin work in `start`.

import "./style.css";
import { defineFeature } from "../../core/feature";

export default defineFeature({
  id: "hub",
});

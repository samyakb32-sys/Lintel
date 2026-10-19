// What a service's pill says under its name (serviceLine and metaLine in
// src/views/integrations.ts): the latest of its card, from the data the card
// is drawn from, and a small line under it that never says the same again.

import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();

const { metaLine, serviceLine } = await import("../src/views/integrations.ts");
const { State } = await import("../src/core/state.ts");

const set = (id, data, more = {}) => {
  State.integrations[id] = { data, error: null, loaded: true, configured: true, ...more };
};
const minutesAgo = (n) => Date.now() - n * 60_000;
const pulse = (over = {}) => ({ login: "me", myPRs: [], toReview: [], mainCI: [], fetchedAt: 0, ...over });

beforeEach(() => {
  State.integrations = {};
});

test("only a service's pill has a line of this kind", () => {
  assert.equal(serviceLine("integration_claude"), null);
  assert.equal(serviceLine("agent_codex"), null);
  assert.equal(serviceLine("session_3"), null);
});

test("not set up, or the last fetch failed: said in red, as on its idle card", () => {
  assert.deepEqual(serviceLine("integration_github"), { text: "Key not configured", tone: "bad" });
  set("integration_github", {}, { error: "GitHub answered 401" });
  assert.deepEqual(serviceLine("integration_github"), { text: "GitHub answered 401", tone: "bad" });
});

test("set up with nothing fetched yet: Connected, and never the bare kind", () => {
  set("integration_resend", {}, { loaded: false });
  assert.deepEqual(serviceLine("integration_resend"), { text: "Connected", tone: "ok" });
  set("integration_n8n", {});
  assert.deepEqual(serviceLine("integration_n8n"), { text: "Connected", tone: "ok" });
});

test("GitHub: a failing default branch, then reviews, then open pull requests, then the calm line", () => {
  const pr = { ci: "success" };
  set("integration_github", { pulse: pulse({ mainCI: [{ ci: "failure" }, { ci: "success" }], toReview: [pr], myPRs: [pr] }) });
  assert.deepEqual(serviceLine("integration_github"), { text: "1 failing on the default branch", tone: "bad" });
  set("integration_github", { pulse: pulse({ mainCI: [{ ci: "success" }], toReview: [pr, pr, pr], myPRs: [pr] }) });
  assert.deepEqual(serviceLine("integration_github"), { text: "3 to review", tone: "warn" });
  set("integration_github", { pulse: pulse({ myPRs: [pr] }) });
  assert.deepEqual(serviceLine("integration_github"), { text: "1 open pull request" });
  set("integration_github", { pulse: pulse({ myPRs: [pr, pr, pr, pr, pr, pr] }) });
  assert.equal(serviceLine("integration_github").text, "6 open pull requests");
  set("integration_github", { pulse: pulse({ mainCI: [{ ci: "success" }] }) });
  assert.deepEqual(serviceLine("integration_github"), { text: "all green", tone: "ok" });
  // No pulse yet, only the numbers of the small card.
  set("integration_github", { totalRepos: 12, totalStars: 40 });
  assert.equal(serviceLine("integration_github").text, "12 repos");
});

test("Vercel: the latest deployment's state and age", () => {
  set("integration_vercel", { deployments: [{ state: "READY", createdAt: minutesAgo(125) }, { state: "ERROR", createdAt: minutesAgo(900) }] });
  assert.deepEqual(serviceLine("integration_vercel"), { text: "Ready · 2h ago", tone: "ok" });
  set("integration_vercel", { deployments: [{ state: "ERROR", createdAt: minutesAgo(5) }] });
  assert.deepEqual(serviceLine("integration_vercel"), { text: "Error · 5m ago", tone: "bad" });
  set("integration_vercel", { deployments: [{ state: "READY", createdAt: Date.now() }] });
  assert.equal(serviceLine("integration_vercel").text, "Ready · just now");
});

test("Resend, Stripe, Notion, Cal.com: the first thing their card says", () => {
  set("integration_resend", { total: 4, emails: [{ to: ["anna@example.com"], subject: "Welcome", lastEvent: "delivered", createdAt: minutesAgo(3) }] });
  assert.deepEqual(serviceLine("integration_resend"), { text: "Welcome · 3m ago", tone: "ok" });
  set("integration_resend", { emails: [{ to: ["anna@example.com"], lastEvent: "bounced", createdAt: minutesAgo(3) }] });
  assert.deepEqual(serviceLine("integration_resend"), { text: "anna · 3m ago", tone: "bad" });

  set("integration_stripe", { balance: 125_050, currency: "eur", payments: [{ status: "succeeded" }] });
  assert.deepEqual(serviceLine("integration_stripe"), { text: "1250.50 EUR", tone: undefined });
  set("integration_stripe", { balance: 0, currency: "usd", payments: [{ status: "failed" }] });
  assert.deepEqual(serviceLine("integration_stripe"), { text: "0.00 USD", tone: "bad" });

  set("integration_notion", { pages: [{ title: "Roadmap", lastEditedAt: minutesAgo(61) }] });
  assert.equal(serviceLine("integration_notion").text, "Roadmap · 1h ago");

  set("integration_calcom", { bookings: [] });
  assert.equal(serviceLine("integration_calcom").text, "No calls scheduled");
  set("integration_calcom", { bookings: [{ start: "2031-03-02T10:00:00", title: "Later" }, { start: "2031-03-01T09:30:00", title: "Intro call" }] });
  assert.match(serviceLine("integration_calcom").text, / · Intro call$/);
});

test("the small line never repeats the line above it, and a service's kind alone is left out", () => {
  // What the owner saw: "Integration" under "Integration".
  assert.equal(metaLine(false, "Integration", undefined, "Integration", ""), "");
  // A service with something to say: the kind alone still says nothing.
  assert.equal(metaLine(false, "3 to review", undefined, "Integration", ""), "");
  // With steps to count it is a line worth having.
  assert.equal(metaLine(false, "Deploy failed", undefined, "Integration", "2/2"), "Integration · 2/2");
  assert.equal(metaLine(false, "Integration", undefined, "Integration", "2/2"), "2/2");
  // A tool or an agent keeps exactly what it showed.
  assert.equal(metaLine(true, "Edits · fsm.ts", "meteor", "Claude Code", "3/7"), "meteor · Claude Code · 3/7");
  assert.equal(metaLine(true, "Claude Code", undefined, "Claude Code", ""), "Claude Code");
});

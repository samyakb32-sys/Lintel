// The requests that wait for an answer (src/core/approvals.ts), driven the way
// they arrive: hook events through the real bridge, clicks through the three
// functions the card's buttons call (src/island/agents.ts). What is checked is
// the rule of CLAUDE.md: nothing is ever answered for the user. A decision
// leaves only on a click, and only for the request the card was drawn for.

import { afterEach, beforeEach, mock, test } from "node:test";
import assert from "node:assert/strict";
import { calls, emit, sent } from "./tauri.mjs";
import { registerHookHandlers } from "../src/island/hooks.ts";
import { answerRequest, decideRequest, returnRequest } from "../src/island/agents.ts";
import { ApprovalQueue, MAX_APPROVALS, byPointer } from "../src/core/approvals.ts";
import { Sessions } from "../src/core/sessions.ts";
import { DEFAULT_SETTINGS, State } from "../src/core/state.ts";

const CLAUDE = "integration_claude";
let asked;
const island = {
  alert: (view) => asked.push(`alert:${view}`),
  setView: (view) => asked.push(`setView:${view}`),
  reveal: () => {},
  dropPin: () => asked.push("dropPin"),
  api: () => ({ notice() {} }),
};
registerHookHandlers(island);

const hook = (payload) => emit("hook", payload);
const task = (id = CLAUDE) => State.tasks.find((t) => t.id === id);
const seconds = (n) => mock.timers.tick(n * 1000);
const queue = () => State.approvals.all().map((a) => a.requestId);
/** Every command that carries a decision to the relay. */
const decisions = () => [...sent("approval_decision"), ...sent("approval_answer")];

const ask = (request_id, session_id = "s1", extra = {}) =>
  hook({
    hook_event_name: "PermissionRequest", request_id, session_id, cwd: `C:\\work\\${session_id}`,
    tool_name: "Bash", tool_input: { command: `run ${request_id}` }, ...extra,
  });

const QUESTION = {
  tool_name: "AskUserQuestion",
  tool_input: { questions: [{ question: "Which?", options: [{ label: "A" }, { label: "B" }] }] },
};

beforeEach(() => {
  mock.timers.enable({ apis: ["setTimeout"] });
  asked = [];
  calls.length = 0;
  State.tasks = [];
  State.focusId = null;
  State.mode = "hidden";
  State.view = "overview";
  State.paused = false;
  State.isPinned = false;
  State.approvals.clear();
  State.sessions = new Sessions();
  State.settings = { ...DEFAULT_SETTINGS };
  State.loadIntegrationTasks();
});

afterEach(() => {
  mock.timers.tick(2 * 3_600_000);
  mock.timers.reset();
});

// ── Three at once ─────────────────────────────────────────────────────────────

test("three approvals at once queue, and each answer goes to its own request", () => {
  ask("r1");
  ask("r2");
  ask("r3");
  assert.deepEqual(queue(), ["r1", "r2", "r3"]);
  assert.deepEqual(sent("approval_ack").map((a) => a.requestId), ["r1", "r2", "r3"]);
  // Taking three requests in decided nothing.
  assert.deepEqual(decisions(), []);
  assert.deepEqual(sent("approval_decline"), []);

  // The card shows the oldest. Allow, clicked on the card drawn for r1.
  assert.equal(State.pendingApproval.command, "Bash · run r1");
  assert.equal(decideRequest("allow", "r1"), true);
  assert.deepEqual(sent("approval_decision"), [{ requestId: "r1", decision: "allow" }]);
  // r2 comes up, undecided: moving up the queue is not an answer.
  assert.deepEqual(queue(), ["r2", "r3"]);
  assert.equal(State.isPinned, true);

  assert.equal(decideRequest("deny", "r2"), true);
  assert.equal(decideRequest("allow", "r3"), true);
  assert.deepEqual(sent("approval_decision"), [
    { requestId: "r1", decision: "allow" },
    { requestId: "r2", decision: "deny" },
    { requestId: "r3", decision: "allow" },
  ]);
  assert.deepEqual(queue(), []);
  assert.equal(State.isPinned, false);
});

test("a click only ever answers the request its card was drawn for", () => {
  ask("r1");
  ask("r2");
  // A click made for r2 while r1 is on the card: not r2's turn, and never r1's answer.
  assert.equal(decideRequest("allow", "r2"), false);
  assert.deepEqual(decisions(), []);
  assert.deepEqual(queue(), ["r1", "r2"]);

  // The double click: the second lands after r1 has left the card, on the
  // button that was drawn for r1. It must not allow r2.
  assert.equal(decideRequest("allow", "r1"), true);
  assert.equal(decideRequest("allow", "r1"), false);
  assert.deepEqual(sent("approval_decision"), [{ requestId: "r1", decision: "allow" }]);
  assert.equal(State.pendingApproval.requestId, "r2");

  // A request nobody has, or no request at all.
  assert.equal(decideRequest("allow", "nope"), false);
  assert.equal(decideRequest("allow", ""), false);
  State.approvals.clear();
  assert.equal(decideRequest("allow", "r2"), false);
  assert.deepEqual(sent("approval_decision"), [{ requestId: "r1", decision: "allow" }]);
});

test("a permission is never answered as a question, nor a question as a permission", () => {
  ask("r1");
  assert.equal(answerRequest({ "Which?": "A" }, [], "r1"), false);
  ask("q1", "s1", QUESTION);
  assert.equal(decideRequest("allow", "r1"), true);
  // The question is on the card now: Allow and Deny mean nothing to it.
  assert.equal(State.pendingApproval.requestId, "q1");
  assert.equal(decideRequest("allow", "q1"), false);
  assert.equal(decideRequest("deny", "q1"), false);
  assert.equal(answerRequest({ "Which?": "B" }, [], "q1"), true);
  assert.deepEqual(sent("approval_decision"), [{ requestId: "r1", decision: "allow" }]);
  assert.deepEqual(sent("approval_answer"), [{ requestId: "q1", answers: { "Which?": "B" }, other: [] }]);
});

test("answer in terminal hands that request back undecided, and the next one comes up", () => {
  ask("r1");
  ask("r2");
  assert.equal(returnRequest("r2"), false);
  assert.equal(returnRequest("r1"), true);
  assert.deepEqual(sent("approval_decline"), [{ requestId: "r1" }]);
  assert.deepEqual(decisions(), []);
  assert.equal(State.pendingApproval.requestId, "r2");
});

// ── What takes a request down without answering it ────────────────────────────

test("time running out, the turn ending and the session ending answer nothing", () => {
  ask("r1");
  ask("r2");
  ask("r3", "s2");
  // r1 and r2 belong to s1's turn: its stop takes both down. s2's stays.
  hook({ hook_event_name: "Stop", session_id: "s1" });
  assert.deepEqual(queue(), ["r3"]);
  assert.deepEqual(sent("approval_decline").map((a) => a.requestId), ["r1", "r2"]);
  seconds(110);
  assert.deepEqual(queue(), []);
  assert.deepEqual(decisions(), []);

  ask("r4", "s2");
  hook({ hook_event_name: "SessionEnd", session_id: "s2" });
  assert.deepEqual(queue(), []);
  assert.deepEqual(decisions(), []);
});

test("a request is matched by agent, session, turn and id", () => {
  // Two agents use the same session id: one's turn ending leaves the other's request.
  ask("r1", "same");
  ask("r2", "same", { meteor_agent: "codex" });
  hook({ hook_event_name: "Stop", session_id: "same", meteor_agent: "codex" });
  assert.deepEqual(queue(), ["r1"]);
  assert.equal(State.pendingApproval.family, CLAUDE);
  // Another session of the same agent changes nothing either.
  hook({ hook_event_name: "Stop", session_id: "other" });
  hook({ hook_event_name: "UserPromptSubmit", session_id: "other", prompt: "hi" });
  assert.deepEqual(queue(), ["r1"]);
  // Its own session's next prompt: the turn that asked is over.
  hook({ hook_event_name: "UserPromptSubmit", session_id: "same", prompt: "never mind" });
  assert.deepEqual(queue(), []);
  assert.deepEqual(decisions(), []);
});

test("a late event of an older turn cannot take a newer turn's card down", () => {
  // Codex numbers its turns. Turn t1 stops, turn t2 asks for a permission.
  const codex = (name, turn_id, extra = {}) =>
    hook({ hook_event_name: name, session_id: "c1", meteor_agent: "codex", turn_id, ...extra });
  codex("UserPromptSubmit", "t1", { prompt: "one" });
  codex("Stop", "t1");
  codex("UserPromptSubmit", "t2", { prompt: "two" });
  ask("r1", "c1", { meteor_agent: "codex", turn_id: "t2" });
  assert.equal(task("agent_codex").state, "approval");
  // t1's events arrive late: another stop, an interrupt, a tool result.
  for (const late of ["Stop", "Interrupt", "StopFailure", "PostToolUse", "PreToolUse"]) codex(late, "t1");
  assert.deepEqual(queue(), ["r1"]);
  assert.equal(task("agent_codex").state, "approval");
  assert.deepEqual(sent("approval_decline"), []);
  // t2's own stop does end it.
  codex("Stop", "t2");
  assert.deepEqual(queue(), []);
});

test("a tool that ran settles the request that waited for it, and only that one", () => {
  ask("r1");
  ask("r2");
  // r2's tool ran: it was answered in the terminal. r1 still waits.
  hook({ hook_event_name: "PostToolUse", session_id: "s1", tool_name: "Bash", tool_input: { command: "run r2" } });
  assert.deepEqual(queue(), ["r1"]);
  assert.deepEqual(sent("approval_decline"), [{ requestId: "r2" }]);
  // Another tool's result, or another session's, settles nothing.
  hook({ hook_event_name: "PostToolUse", session_id: "s1", tool_name: "Read", tool_input: { file_path: "a" } });
  hook({ hook_event_name: "PostToolUse", session_id: "s9", tool_name: "Bash", tool_input: { command: "run r1" } });
  assert.deepEqual(queue(), ["r1"]);
  assert.equal(task().state, "approval");

  // Two identical requests: the result that follows an Allow given here is that
  // Allow's own, and does not take the twin down.
  ask("twin-a", "s1", { tool_input: { command: "ls" } });
  ask("twin-b", "s1", { tool_input: { command: "ls" } });
  decideRequest("deny", "r1");
  decideRequest("allow", "twin-a");
  hook({ hook_event_name: "PostToolUse", session_id: "s1", tool_name: "Bash", tool_input: { command: "ls" } });
  assert.deepEqual(queue(), ["twin-b"]);
  assert.deepEqual(decisions().map((d) => d.requestId), ["r1", "twin-a"]);
});

test("a question dismissed in the terminal takes its card down", () => {
  ask("q1", "s1", QUESTION);
  State.view = "question";
  hook({ hook_event_name: "PostToolUseFailure", session_id: "s1", tool_name: "AskUserQuestion", tool_input: QUESTION.tool_input });
  assert.equal(State.pendingApproval, null);
  assert.deepEqual(decisions(), []);
});

test("the pill of the request on the card is in front, and the one you were on comes back last", () => {
  State.setFocus("integration_n8n");
  ask("r1", "s1");
  ask("r2", "s2");
  // Two sessions of one agent: each its own pill, each its own request.
  const [first, second] = State.approvals.all().map((a) => a.pillId);
  assert.equal(first, CLAUDE);
  assert.notEqual(second, CLAUDE);
  assert.equal(State.focusId, first);
  assert.equal(task(second).state, "approval");
  decideRequest("allow", "r1");
  assert.equal(State.focusId, second);
  assert.equal(task(first).state, "working");
  decideRequest("deny", "r2");
  assert.equal(State.focusId, "integration_n8n");
});

// ── The queue alone ───────────────────────────────────────────────────────────

test("the queue is first in first out, five at most, and never holds a request twice", () => {
  const q = new ApprovalQueue();
  const info = (requestId, extra = {}) =>
    ({ requestId, family: "f", sessionId: "s", turn: 1, pillId: "f", tool: "Bash", command: "ls", ...extra });
  assert.equal(q.head, null);
  for (let i = 1; i <= MAX_APPROVALS; i++) assert.equal(q.push(info(`r${i}`)), "queued");
  assert.equal(q.push(info("r6")), "full");
  assert.equal(q.push(info("r3")), "known");
  assert.equal(q.size, 5);
  assert.equal(q.head.requestId, "r1");
  // Taken by id: from the middle, the order of the rest is kept.
  assert.equal(q.take("r3").requestId, "r3");
  assert.equal(q.take("r3"), null);
  assert.deepEqual(q.all().map((a) => a.requestId), ["r1", "r2", "r4", "r5"]);
  // A turn takes its own and older ones, of its own agent and session only.
  q.push(info("next", { turn: 2 }));
  q.push(info("else", { sessionId: "t" }));
  assert.deepEqual(q.ofTurn("f", "s", 1).map((a) => a.requestId), ["r1", "r2", "r4", "r5"]);
  assert.deepEqual(q.ofTurn("f", "s", 2).length, 5);
  assert.deepEqual(q.ofTurn("g", "s", 9), []);
});

test("a click made by a key or by code answers nothing: only a pointer press counts", () => {
  assert.equal(byPointer({ detail: 1 }), true); // a mouse click
  assert.equal(byPointer({ detail: 2 }), true); // the second of a double click
  assert.equal(byPointer({ detail: 0 }), false); // Enter or Space on the focused button, or element.click()
  assert.equal(byPointer(), false);
});

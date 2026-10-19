// One pill per session, and what an event means for its turn
// (src/core/sessions.ts): first as the plain rules, then through the hook
// handler with the clock under control.

import { afterEach, beforeEach, mock, test } from "node:test";
import assert from "node:assert/strict";
import { calls, emit, sent } from "./tauri.mjs";
import { registerHookHandlers } from "../src/island/hooks.ts";
import {
  MAX_SESSIONS, QUIT_AFTER_MS, SILENT_MS, Sessions, isClear, judge, mayQuit, modelLabel,
} from "../src/core/sessions.ts";
import { DEFAULT_SETTINGS, State, familyOf, needsYou } from "../src/core/state.ts";

const CLAUDE = "integration_claude";
let asked;
let notices;
const island = {
  alert: (view) => asked.push(`alert:${view}`),
  setView: (view) => asked.push(`setView:${view}`),
  reveal: () => {},
  dropPin: () => {},
  api: () => ({ notice: (n) => notices.push(`${n.title}: ${n.detail}`) }),
};
registerHookHandlers(island);

const hook = (payload) => emit("hook", payload);
const task = (id = CLAUDE) => State.tasks.find((t) => t.id === id);
const pillOf = (id, family = CLAUDE) => State.sessions.find(family, id)?.pillId;
const minutes = (n) => mock.timers.tick(n * 60_000);
const finishes = () => asked.filter((a) => a.endsWith(":finished")).length;

beforeEach(() => {
  // The clock too: half an hour of silence is told by the date.
  mock.timers.enable({ apis: ["setTimeout", "Date"], now: new Date(2026, 9, 12, 9, 0) });
  asked = [];
  notices = [];
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

// ── The rules alone ───────────────────────────────────────────────────────────

const fresh = () => new Sessions().touch("f", "s", 0).session;
const said = (s, ...events) => events.map((e) => judge(s, typeof e === "string" ? { name: e } : e));

test("the first session keeps the agent's pill, every other one gets its own", () => {
  const all = new Sessions();
  const a = all.touch(CLAUDE, "a", 0).session;
  const b = all.touch(CLAUDE, "b", 1).session;
  const c = all.touch(CLAUDE, "c", 2).session;
  const codex = all.touch("agent_codex", "a", 3).session;
  assert.equal(a.pillId, CLAUDE);
  assert.notEqual(b.pillId, CLAUDE);
  assert.notEqual(c.pillId, b.pillId);
  // Another agent's first session has its own agent's pill, same session id or not.
  assert.equal(codex.pillId, "agent_codex");
  // The same session again is the same pill.
  assert.equal(all.touch(CLAUDE, "b", 9).session, b);
  // Once the first has ended, the next new session takes the agent's pill.
  assert.equal(all.end(CLAUDE, "a"), a);
  assert.equal(all.touch(CLAUDE, "d", 10).session.pillId, CLAUDE);
  assert.equal(all.find(CLAUDE, "b").pillId, b.pillId);
});

test("twelve sessions at most, and half an hour of silence ends one", () => {
  const all = new Sessions();
  for (let i = 0; i < MAX_SESSIONS; i++) assert.deepEqual(all.touch("f", `s${i}`, i).dropped, []);
  // The thirteenth takes the place of the one heard longest ago.
  all.touch("f", "s0", 100);
  const { dropped } = all.touch("f", "new", 101);
  assert.deepEqual(dropped.map((s) => s.id), ["s1"]);
  assert.equal(all.list().length, MAX_SESSIONS);

  const quiet = new Sessions();
  quiet.touch("f", "old", 0);
  quiet.touch("f", "live", SILENT_MS - 1);
  assert.deepEqual(quiet.prune(SILENT_MS - 1), []);
  assert.deepEqual(quiet.prune(SILENT_MS).map((s) => s.id), ["old"]);
  assert.equal(quiet.nextPrune(), 2 * SILENT_MS - 1);
  assert.equal(new Sessions().nextPrune(), null);
});

test("a turn finishes once: the second stop of the same turn is late", () => {
  const s = fresh();
  assert.deepEqual(
    said(s, { name: "UserPromptSubmit", prompt: "go" }, "PreToolUse", "PostToolUse", "Stop", "Stop", "PostToolUse", "SubagentStop"),
    ["open", "open", "open", "finish", "late", "late", "late"],
  );
  // The next prompt is the next turn, and finishes in its turn.
  assert.deepEqual(said(s, { name: "UserPromptSubmit", prompt: "more" }, "Stop"), ["open", "finish"]);
  assert.equal(s.turn, 2);
});

test("an agent that only ever says it stopped finishes every time", () => {
  // No prompt event (Hermes, OpenCode): each stop is a turn of its own.
  const s = fresh();
  assert.deepEqual(said(s, "Stop", "Stop", "PreToolUse", "Stop"), ["finish", "finish", "open", "finish"]);
});

test("a stop with background tasks still running is not the end of the turn", () => {
  const s = fresh();
  said(s, { name: "UserPromptSubmit", prompt: "go" });
  assert.equal(judge(s, { name: "Stop", background: 2 }), "background");
  assert.equal(s.open, true);
  // The subagents write on: shown as theirs, never the session's last words.
  assert.equal(judge(s, { name: "PreToolUse", subagent: true }), "sub");
  assert.equal(judge(s, { name: "SubagentStop" }), "sub");
  assert.equal(judge(s, { name: "SubagentStop" }), "sub");
  assert.equal(s.background, 0);
  // The CLI wakes the agent with a prompt of its own: the same turn goes on.
  assert.equal(judge(s, { name: "UserPromptSubmit", prompt: "<task-notification>done</task-notification>" }), "open");
  assert.equal(s.turn, 1);
  // And the one finish of the turn is its real end.
  assert.equal(judge(s, { name: "Stop", background: 0 }), "finish");
});

test("a finish announced early is not announced again when the turn wakes", () => {
  // A CLI that does not say what runs in the background: the first stop is
  // announced, and the stop after the wake is the same turn's.
  const s = fresh();
  said(s, { name: "UserPromptSubmit", prompt: "go" });
  assert.equal(judge(s, { name: "Stop" }), "finish");
  // A subagent of the turn that is over writes on: ignored.
  assert.equal(judge(s, { name: "PreToolUse", subagent: true }), "late");
  assert.deepEqual(said(s, { name: "UserPromptSubmit", prompt: "<task-notification>x" }, "PreToolUse", "Stop"), ["open", "open", "quiet"]);
});

test("/clear ends a turn with nothing to announce", () => {
  assert.equal(isClear("/clear"), true);
  assert.equal(isClear("  /clear  "), true);
  assert.equal(isClear("/clear now"), true);
  for (const not of ["/clearance", "clear", "please /clear", "", undefined]) assert.equal(isClear(not), false);
  const s = fresh();
  said(s, { name: "UserPromptSubmit", prompt: "go" }, "PreToolUse");
  assert.equal(judge(s, { name: "UserPromptSubmit", prompt: "/clear" }), "end");
  assert.equal(s.open, false);
  assert.equal(s.turn, 1);
  // Whatever the cleared turn still sends is late; a session that starts again is not a turn.
  assert.deepEqual(said(s, "Stop", "PostToolUse"), ["late", "late"]);
  assert.equal(judge(s, { name: "SessionStart" }), "open");
  assert.equal(s.open, false);
});

test("an interrupt or a failure closes the turn; a permission request is never late", () => {
  const s = fresh();
  said(s, { name: "UserPromptSubmit", prompt: "go" });
  assert.equal(judge(s, { name: "Interrupt" }), "end");
  assert.equal(judge(s, { name: "Stop" }), "late");
  assert.equal(judge(s, { name: "PermissionRequest" }), "open");
  assert.equal(s.open, true);
  assert.equal(judge(s, { name: "StopFailure" }), "error");
  assert.equal(judge(s, { name: "PermissionRequest", subagent: true }), "open");
});

test("an agent's own turn ids tell an old turn's event from the current one's", () => {
  const s = fresh();
  assert.equal(judge(s, { name: "UserPromptSubmit", turnId: "t1", prompt: "one" }), "open");
  assert.equal(judge(s, { name: "Stop", turnId: "t1" }), "finish");
  assert.equal(judge(s, { name: "UserPromptSubmit", turnId: "t2", prompt: "two" }), "open");
  for (const name of ["Stop", "Interrupt", "StopFailure", "PreToolUse", "PostToolUse", "UserPromptSubmit"]) {
    assert.equal(judge(s, { name, turnId: "t1", prompt: "one" }), "late", name);
  }
  assert.equal(s.turn, 2);
  assert.equal(s.open, true);
  assert.equal(judge(s, { name: "Stop", turnId: "t2" }), "finish");
});

test("a model reads as its name, and an app the relay started leaves only when all is quiet", () => {
  assert.equal(modelLabel("claude-opus-5-5"), "Opus 5.5");
  assert.equal(modelLabel("claude-sonnet-5"), "Sonnet 5");
  assert.equal(modelLabel("claude-haiku-4-5-20251001"), "Haiku 4.5");
  assert.equal(modelLabel("gpt-6-luna"), "gpt-6-luna");
  assert.equal(modelLabel(""), "");

  const quiet = { autoLaunched: true, now: QUIT_AFTER_MS, lastHeard: 0, lastUsed: 0, busy: false, waiting: 0, islandShown: false };
  assert.equal(mayQuit(quiet), true);
  // Never an app the user started, an agent at work, a card that waits, an island in use, or too soon.
  assert.equal(mayQuit({ ...quiet, autoLaunched: false }), false);
  assert.equal(mayQuit({ ...quiet, busy: true }), false);
  assert.equal(mayQuit({ ...quiet, waiting: 1 }), false);
  assert.equal(mayQuit({ ...quiet, islandShown: true }), false);
  assert.equal(mayQuit({ ...quiet, lastHeard: 1 }), false);
  assert.equal(mayQuit({ ...quiet, lastUsed: 1 }), false);
  assert.equal(mayQuit({ ...quiet, lastHeard: null }), true);
});

// ── Through the hook handler ──────────────────────────────────────────────────

test("two sessions of one agent stop wiping each other's card and finish", () => {
  hook({ hook_event_name: "UserPromptSubmit", session_id: "a", cwd: "C:\\work\\alpha", prompt: "one" });
  hook({ hook_event_name: "UserPromptSubmit", session_id: "b", cwd: "C:\\work\\beta", prompt: "two" });
  const [a, b] = [pillOf("a"), pillOf("b")];
  assert.equal(a, CLAUDE);
  assert.notEqual(b, CLAUDE);
  // Each has its own name, folder, steps, and looks like its agent's pill.
  assert.equal(task(a).name, "alpha");
  assert.equal(task(b).name, "beta");
  assert.equal(task(b).sessionCwd, "C:\\work\\beta");
  assert.equal(task(b).sessionId, "b");
  assert.equal(familyOf(task(b)), CLAUDE);
  assert.equal(task(b).color, task(a).color);
  assert.deepEqual(task(a).steps, ["one"]);
  assert.deepEqual(task(b).steps, ["two"]);

  // b finishes: a goes on working, untouched, and stays so past b's rest.
  hook({ hook_event_name: "PreToolUse", session_id: "a", tool_name: "Bash", tool_input: { command: "npm test" } });
  hook({ hook_event_name: "Stop", session_id: "b", last_assistant_message: "Beta is done." });
  assert.equal(task(b).state, "finished");
  assert.equal(task(b).finalLine, "Beta is done.");
  assert.equal(task(a).state, "working");
  assert.equal(task(a).finalLine ?? null, null);
  mock.timers.tick(5200);
  assert.equal(task(a).state, "working");
  assert.equal(task(b).state, "idle");

  // b's session ends: its pill goes, a's stays with everything it had.
  hook({ hook_event_name: "SessionEnd", session_id: "b" });
  assert.equal(task(b), undefined);
  assert.deepEqual(task(a).steps, ["one", "Runs · npm test"]);
  assert.equal(task(a).name, "alpha");
});

test("a second session of any agent gets its own pill, and the agent's own stays its first session's", () => {
  hook({ hook_event_name: "PreToolUse", session_id: "x", meteor_agent: "codex", cwd: "C:\\w\\one", tool_name: "shell" });
  hook({ hook_event_name: "PreToolUse", session_id: "y", meteor_agent: "codex", cwd: "C:\\w\\two", tool_name: "shell" });
  const y = pillOf("y", "agent_codex");
  assert.equal(pillOf("x", "agent_codex"), "agent_codex");
  assert.notEqual(y, "agent_codex");
  assert.equal(task(y).name, "two");
  assert.equal(task(y).source, "agent");
  assert.equal(familyOf(task(y)), "agent_codex");
  assert.equal(task(y).lastTool, "shell");
});

test("a pill leaves after thirty silent minutes, and not before", () => {
  hook({ hook_event_name: "PreToolUse", session_id: "a", cwd: "C:\\w\\a", tool_name: "Bash" });
  hook({ hook_event_name: "PreToolUse", session_id: "b", cwd: "C:\\w\\b", tool_name: "Bash" });
  const b = pillOf("b");
  minutes(20);
  // a speaks, b stays silent.
  hook({ hook_event_name: "PreToolUse", session_id: "a", tool_name: "Bash" });
  minutes(9);
  assert.notEqual(task(b), undefined);
  // The next event of anyone, half an hour after b last spoke, takes b's pill away.
  minutes(1);
  hook({ hook_event_name: "PreToolUse", session_id: "a", tool_name: "Bash" });
  assert.equal(task(b), undefined);
  assert.equal(State.sessions.find(CLAUDE, "b"), null);
  // With nobody speaking at all, the timer does it: a's own pill is the main one and goes back to rest.
  assert.equal(task().state, "working");
  minutes(30);
  assert.equal(State.sessions.list().length, 0);
  assert.equal(task().state, "idle");
  assert.equal(task().name, "Claude Code");
});

test("a background task still running at the stop keeps the pill working, and the turn finishes once", () => {
  State.focusId = CLAUDE;
  hook({ hook_event_name: "UserPromptSubmit", session_id: "a", prompt: "go" });
  hook({
    hook_event_name: "Stop", session_id: "a", last_assistant_message: "Two agents are on it.",
    background_tasks: [{ type: "subagent" }, { type: "shell" }],
  });
  assert.equal(task().state, "working");
  assert.equal(task().steps.at(-1), "In background: 2");
  assert.equal(task().finalLine ?? null, null);
  assert.equal(finishes(), 0);
  assert.deepEqual(sent("approval_decline"), []);
  // It stays working: no return to idle was armed.
  mock.timers.tick(10_000);
  assert.equal(task().state, "working");

  // A background subagent's tools are steps of its own and never the last tool or the last words.
  hook({ hook_event_name: "PreToolUse", session_id: "a", agent_id: "sub-1", tool_name: "Grep", tool_input: { pattern: "x" } });
  assert.equal(task().steps.at(-1), "↳ Searches");
  assert.equal(task().lastTool ?? null, null);
  // The CLI's own helpers are not the session's work.
  hook({ hook_event_name: "SubagentStop", session_id: "a", agent_type: "" });
  assert.equal(State.sessions.find(CLAUDE, "a").background, 2);

  // The agent wakes and gives its real answer: finished, once.
  hook({ hook_event_name: "UserPromptSubmit", session_id: "a", prompt: "<task-notification>done</task-notification>" });
  assert.notEqual(task().steps.at(-1), "<task-notification>done</task-notification>");
  hook({ hook_event_name: "Stop", session_id: "a", last_assistant_message: "All **done**.", background_tasks: [] });
  assert.equal(task().state, "finished");
  assert.equal(task().finalLine, "All done.");
  assert.equal(task().finalText, "All **done**.");
  assert.equal(finishes(), 1);
  // A second stop of the same turn, and a late tool result: nothing more.
  hook({ hook_event_name: "Stop", session_id: "a", last_assistant_message: "All **done**." });
  hook({ hook_event_name: "PostToolUse", session_id: "a", tool_name: "Bash" });
  assert.equal(finishes(), 1);
  assert.equal(task().state, "finished");
});

test("a subagent writing on after the answer does not replace it", () => {
  hook({ hook_event_name: "UserPromptSubmit", session_id: "a", prompt: "go" });
  hook({ hook_event_name: "Stop", session_id: "a", last_assistant_message: "Here is the answer." });
  hook({ hook_event_name: "PreToolUse", session_id: "a", agent_id: "sub-1", tool_name: "Bash", tool_input: { command: "cd /tmp" } });
  hook({ hook_event_name: "PostToolUse", session_id: "a", agent_id: "sub-1", tool_name: "Bash" });
  assert.equal(task().state, "finished");
  assert.equal(task().finalLine, "Here is the answer.");
  assert.equal(task().steps.at(-1), "Here is the answer.");
});

test("/clear ends the turn: no thinking for ever, no finished card", () => {
  State.focusId = CLAUDE;
  hook({ hook_event_name: "UserPromptSubmit", session_id: "a", cwd: "C:\\w\\proj", prompt: "work" });
  hook({ hook_event_name: "PreToolUse", session_id: "a", tool_name: "Bash", tool_input: { command: "ls" } });
  hook({ hook_event_name: "UserPromptSubmit", session_id: "a", prompt: "/clear" });
  assert.equal(task().state, "idle");
  assert.deepEqual(task().steps, []);
  assert.equal(finishes(), 0);
  // What Claude Code sends around it: the old session ends, a new one starts.
  hook({ hook_event_name: "SessionEnd", session_id: "a", reason: "clear" });
  asked = [];
  hook({ hook_event_name: "SessionStart", session_id: "a2", cwd: "C:\\w\\proj", source: "clear" });
  assert.equal(pillOf("a2"), CLAUDE);
  assert.equal(task().state, "idle");
  assert.deepEqual(asked, []);
  assert.equal(finishes(), 0);
  // The next real prompt is a turn like any other.
  hook({ hook_event_name: "UserPromptSubmit", session_id: "a2", prompt: "again" });
  hook({ hook_event_name: "Stop", session_id: "a2", message: "ok" });
  assert.equal(finishes(), 1);
});

test("a session that waits in its terminal says so on its pill, with a notice and no card", () => {
  hook({ hook_event_name: "PreToolUse", session_id: "a", cwd: "C:\\w\\alpha", tool_name: "Bash" });
  hook({ hook_event_name: "PreToolUse", session_id: "b", cwd: "C:\\w\\beta", tool_name: "Bash" });
  const b = pillOf("b");
  hook({ hook_event_name: "Notification", session_id: "b", notification_type: "permission_prompt", message: "Claude needs your permission to use Bash" });
  assert.equal(task(b).state, "approval");
  assert.equal(needsYou(task(b)), true);
  assert.equal(needsYou(task()), false);
  assert.deepEqual(notices, ["beta: Waiting for you"]);
  // Nothing was put on a card and nothing was answered.
  assert.equal(State.pendingApproval, null);
  assert.deepEqual([...sent("approval_decision"), ...sent("approval_ack"), ...sent("approval_decline")], []);
  // It goes back to work when the session does.
  hook({ hook_event_name: "PostToolUse", session_id: "b", tool_name: "Bash" });
  assert.equal(needsYou(task(b)), false);
  // An idle prompt is not a session that needs you.
  hook({ hook_event_name: "Notification", session_id: "a", notification_type: "idle_prompt", message: "Claude is waiting for your input" });
  assert.equal(needsYou(task()), false);
});

test("the session chips come from what the agent says: mode, effort, model", () => {
  hook({
    hook_event_name: "SessionStart", session_id: "a", cwd: "C:\\w\\proj",
    permission_mode: "acceptEdits", model: "claude-opus-5-5",
  });
  assert.equal(task().mode, "acceptEdits");
  assert.equal(task().model, "Opus 5.5");
  // The status line relay names the session, the model and the effort.
  hook({
    hook_event_name: "StatusLine", session_id: "a",
    model: { id: "claude-sonnet-5", display_name: "Sonnet 5" }, effort: { level: "high" },
  });
  assert.equal(task().model, "Sonnet 5");
  assert.equal(task().effort, "high");
  // A session nobody has seen gets nothing, and no pill is made for a status line.
  const before = State.tasks.length;
  hook({ hook_event_name: "StatusLine", session_id: "zz", model: { id: "claude-haiku-4-5" } });
  assert.equal(State.tasks.length, before);
  // The session's end takes them with it.
  hook({ hook_event_name: "SessionEnd", session_id: "a" });
  assert.equal(task().mode ?? null, null);
  assert.equal(task().model ?? null, null);
});

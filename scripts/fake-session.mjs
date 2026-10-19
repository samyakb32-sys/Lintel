// Plays pretend agent sessions through the real relay, so the island, the
// relay and the pipe can be tried end to end without an agent. Adapted from
// upstream `windows/scripts/fake-session.mjs` (pull request 105).
//
//   npm run fake-session                the whole session: start, tool steps, a
//                                       permission request, a question, finish
//   npm run fake-session -- permission  only the permission request
//   npm run fake-session -- question    only the question (three of them: pick
//                                       one, pick several, and "Other…")
//   npm run fake-session -- two         two sessions of one agent at once: two
//                                       pills, a session list, one finishes
//                                       while the other works
//   npm run fake-session -- three       three permission requests at once: they
//                                       queue ("1 of 3"), each answer goes to
//                                       its own request
//   npm run fake-session -- background  a turn that stops with background tasks
//                                       still running: no "finished" until the
//                                       real end, and only one
//   npm run fake-session -- clear       a turn, then /clear: the pill rests,
//                                       nothing is announced
//   npm run fake-session -- desktop     a session shaped like one of the Claude
//                                       desktop app: its permission card, its
//                                       question card, "Open Claude"
//   npm run fake-session -- markdown    a finished turn with a long Markdown
//                                       answer (bold, code, links good and bad,
//                                       raw HTML) and the session chips
//   npm run fake-session -- waiting     a second session that waits in its
//                                       terminal: amber ring, a notice, no card
//   npm run fake-session -- timeout     a permission request nobody answers:
//                                       how long the relay waited, and that it
//                                       printed nothing (about 110 s)
//
// Every event is piped into meteor-hook exactly as Claude Code would do it.
// Nothing is ever executed: the "command" in a permission request is only
// text for the card. For the events that wait, the script prints what Claude
// Code would receive: the decision JSON, or nothing at all when the island did
// not answer and the terminal would have asked instead. The script answers
// nothing itself: every Allow, Deny and option is a click on the island.
//
// The relay is the one the app deployed (%LOCALAPPDATA%\<app>\bin), the only
// one the app raises a card for; start the app once first. METEOR_HOOK names
// another one, for instance a fresh build in target\release, whose permission
// requests the app must refuse.

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// The codename, from the one place that holds it (src/core/brand.ts).
const brand = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "src", "core", "brand.ts"), "utf8");
const slug = /slug:\s*"([^"]+)"/.exec(brand)?.[1] ?? "meteor";

const hook = process.env.METEOR_HOOK || join(process.env.LOCALAPPDATA ?? "", slug, "bin", `${slug}-hook.exe`);
if (!existsSync(hook)) {
  console.error(`${hook} not found. Start the app once: it puts the relay there.`);
  process.exit(1);
}

const stamp = Date.now();
const cwd = process.cwd();
/** One pretend session: its id, its folder, and the environment its relay runs in. */
const session = (tag, folder = cwd, env = {}) => ({ session_id: `fake-${tag}-${stamp}`, cwd: folder, env });
const MAIN = session("a");
/** What every Claude Code event says about its session: the chips. */
const RUNS_WITH = { permission_mode: "acceptEdits", model: "claude-opus-5-5", effort: "high" };

const permission = (command = "rm -rf ./build  # fake: nothing runs") => ({
  hook_event_name: "PermissionRequest",
  tool_name: "Bash",
  tool_input: { command },
  permission_suggestions: [{ type: "addRules", rules: [{ toolName: "Bash", ruleContent: "rm:*" }], behavior: "allow" }],
});

const QUESTION = {
  hook_event_name: "PermissionRequest",
  tool_name: "AskUserQuestion",
  tool_input: {
    questions: [
      {
        question: "Which folder should the fake build go to?",
        header: "Folder",
        multiSelect: false,
        options: [
          { label: "dist", description: "The usual one" },
          { label: "build", description: "The one just tidied" },
        ],
      },
      {
        question: "What should the fake build include?",
        header: "Extras",
        multiSelect: true,
        options: [
          { label: "Tests", description: "Run them first" },
          { label: "Docs", description: "Build the pages" },
          { label: "Lint", description: "Check the style" },
        ],
      },
      {
        question: "Who is the fake build for? (try Other…)",
        header: "Audience",
        multiSelect: false,
        options: [
          { label: "Me", description: "Local only" },
          { label: "Everyone", description: "A release" },
        ],
      },
    ],
  },
};

const LONG_ANSWER = [
  "## Build folder tidied",
  "",
  "**Done.** Removed `build/` and kept `dist/`. See [the log](https://example.com/log) for the details.",
  "",
  "- 14 files removed",
  "- 2 kept, both under `dist/assets`",
  "",
  "None of these may become a link or markup: [bad](javascript:alert(1)), [file](file:///C:/Windows/win.ini),",
  '<img src=x onerror="alert(1)"> and <script>alert(2)</script>. An unclosed `code span and **bold stay as typed.',
  "",
  "```",
  "npm run build   # fake: nothing ran",
  "```",
  "",
  "Next: run the tests, then tag the release.",
].join("\n");

const tool = (tool_name, tool_input, extra = {}) => [
  { hook_event_name: "PreToolUse", tool_name, tool_input, ...extra },
  { hook_event_name: "PostToolUse", tool_name, tool_input, ...extra },
];

const SESSION = [
  { hook_event_name: "SessionStart", source: "startup", ...RUNS_WITH },
  { hook_event_name: "UserPromptSubmit", prompt: "Tidy up the build folder", ...RUNS_WITH },
  ...tool("Read", { file_path: join(cwd, "package.json") }),
  ...tool("Grep", { pattern: "build" }),
  permission(),
  QUESTION,
  { hook_event_name: "Stop", last_assistant_message: "**Build folder tidied.** Ran `rm -rf build`, see [the log](https://example.com/log)." },
];

/** One relay run: JSON on stdin, whatever the relay prints on stdout, and how long it took. */
function send(event, who = MAIN) {
  const { env, ...ids } = who;
  const started = Date.now();
  return new Promise((done) => {
    const child = spawn(hook, [event.hook_event_name], { stdio: ["pipe", "pipe", "inherit"], env: { ...process.env, ...env } });
    let out = "";
    child.stdout.on("data", (chunk) => (out += chunk));
    child.on("error", (err) => done({ code: -1, out: String(err), ms: Date.now() - started }));
    child.on("close", (code) => done({ code, out: out.trim(), ms: Date.now() - started }));
    child.stdin.end(JSON.stringify({ ...ids, ...event }));
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const waits = (event) => event.hook_event_name === "PermissionRequest";

/** Says what an event did; for one that waits, what Claude Code would get back. */
function report(event, { code, out, ms }, who = MAIN) {
  const name = `${event.hook_event_name}${event.tool_name ? ` (${event.tool_name})` : ""} [${who.session_id.split("-")[1]}]`;
  if (waits(event)) {
    console.log(out ? `${name}: Claude Code would get: ${out}` : `${name}: no answer after ${(ms / 1000).toFixed(1)} s: Claude Code would ask in the terminal`);
  } else {
    console.log(`${name}${code === 0 ? "" : ` (exit ${code})`}`);
  }
}

/** Events one after the other, 0.9 s apart, as a turn goes. */
async function play(events, who = MAIN, pause = 900) {
  for (const event of events) {
    if (waits(event)) console.log(`${event.hook_event_name} (${event.tool_name}): waiting for a click on the island…`);
    report(event, await send(event, who), who);
    await sleep(pause);
  }
}

const SCENES = {
  session: () => play(SESSION),
  permission: () => play([permission()]),
  question: () => play([QUESTION]),

  // Two sessions of one agent: the first keeps the agent's pill, the second
  // gets its own. With both alive the focused pill lists them. B finishes while
  // A still works: A's steps and state must not move, and only B's pill says so.
  async two() {
    const [a, b] = [session("a", join(cwd, "src")), session("b", join(cwd, "docs"))];
    await play([{ hook_event_name: "SessionStart", source: "startup", ...RUNS_WITH }, { hook_event_name: "UserPromptSubmit", prompt: "Refactor the island" }], a);
    await play([{ hook_event_name: "SessionStart", source: "startup", ...RUNS_WITH, model: "claude-sonnet-5" }, { hook_event_name: "UserPromptSubmit", prompt: "Write the docs" }], b);
    for (let i = 0; i < 3; i++) {
      await play(tool("Edit", { file_path: join(a.cwd, `island-${i}.ts`), old_string: "a", new_string: "b" }), a, 500);
      await play(tool("Read", { file_path: join(b.cwd, `page-${i}.md`) }), b, 500);
    }
    await play([{ hook_event_name: "Stop", last_assistant_message: "The docs are written." }], b);
    console.log("B has finished; A still works for ten seconds. Then both sessions end and the second pill goes.");
    await play(tool("Bash", { command: "npm test" }), a, 5000);
    await play([{ hook_event_name: "Stop", last_assistant_message: "Refactor done, tests pass." }], a, 6000);
    await play([{ hook_event_name: "SessionEnd" }], b);
    await play([{ hook_event_name: "SessionEnd" }], a);
  },

  // Three requests at once, from three relays. They queue in the order they
  // arrived; the card says "1 of 3". Answer them differently (Allow, Deny,
  // Allow) and check below that each relay got its own answer.
  async three() {
    await play([{ hook_event_name: "UserPromptSubmit", prompt: "Three things at once" }]);
    const commands = ["rm -rf ./one  # fake", "rm -rf ./two  # fake", "rm -rf ./three  # fake"];
    console.log("Three permission requests are waiting. Answer each on the island: Allow, Deny, Allow.");
    const running = [];
    for (const command of commands) {
      const event = permission(command);
      running.push(send(event).then((result) => console.log(`  ${command.padEnd(24)} → ${result.out || "no answer"}`)));
      // Far enough apart that the order they reach the app in is the order above.
      await sleep(300);
    }
    await Promise.all(running);
    await play([{ hook_event_name: "Stop", last_assistant_message: "All three were asked." }]);
  },

  // The agent says its piece while two tasks still run in the background: the
  // pill must stay "working" ("In background: 2"), with no finished card and no
  // sound. A subagent's tools show as "↳ …". Then the CLI wakes the agent and
  // the real answer comes: finished, once. The last two events are late (a
  // second stop, a tool result) and must change nothing.
  async background() {
    await play([
      { hook_event_name: "UserPromptSubmit", prompt: "Explore in the background" },
      { hook_event_name: "Stop", last_assistant_message: "Two agents are on it.", background_tasks: [{ type: "subagent" }, { type: "shell" }] },
    ]);
    console.log("Stopped with two background tasks: the pill must still say it works.");
    await play([...tool("Grep", { pattern: "TODO" }, { agent_id: "sub-1", agent_type: "Explore" }), { hook_event_name: "SubagentStop", agent_type: "Explore" }], MAIN, 1500);
    await play([
      { hook_event_name: "UserPromptSubmit", prompt: "<task-notification>The background tasks are done.</task-notification>" },
      { hook_event_name: "Stop", last_assistant_message: "**Explored.** Three TODOs left in `src/island`.", background_tasks: [] },
    ]);
    console.log("That was the one finish. Two late events follow and must change nothing.");
    await play([{ hook_event_name: "Stop", last_assistant_message: "**Explored.** Three TODOs left in `src/island`." }, ...tool("Bash", { command: "late" }).slice(1)]);
  },

  // A turn at work, then /clear typed in the terminal: the pill rests at once,
  // its steps go, nothing is announced, and it never says "thinking" again
  // until the next real prompt.
  async clear() {
    const next = session("a2");
    await play([{ hook_event_name: "UserPromptSubmit", prompt: "Start something" }, ...tool("Read", { file_path: join(cwd, "README.md") }).slice(0, 1)]);
    await play([{ hook_event_name: "UserPromptSubmit", prompt: "/clear" }, { hook_event_name: "SessionEnd", reason: "clear" }]);
    await play([{ hook_event_name: "SessionStart", source: "clear" }], next);
    console.log("Cleared: the pill must rest, with no finished card. A new prompt follows in five seconds.");
    await sleep(5000);
    await play([{ hook_event_name: "UserPromptSubmit", prompt: "A fresh start" }, { hook_event_name: "Stop", last_assistant_message: "Fresh and done." }], next);
  },

  // The relay tags a session of the Claude desktop app from its environment.
  // It must get a permission card and a question card like a terminal session,
  // and its finished card must say "Open Claude".
  async desktop() {
    const desktop = session("desktop", cwd, { CLAUDE_CODE_ENTRYPOINT: "claude-desktop" });
    await play([
      { hook_event_name: "SessionStart", source: "startup", ...RUNS_WITH },
      { hook_event_name: "UserPromptSubmit", prompt: "From the desktop app" },
      permission("npm publish  # fake: nothing runs"),
      QUESTION,
      { hook_event_name: "Stop", last_assistant_message: "Done from the desktop app." },
    ], desktop);
    await sleep(6000);
    await play([{ hook_event_name: "SessionEnd" }], desktop);
  },

  // The finished card with an answer one line does not hold: it must scroll
  // inside the card, with bold, code and the https link only. The javascript:
  // and file: links, the <img> and the <script> must show as plain characters.
  markdown: () =>
    play([
      { hook_event_name: "SessionStart", source: "startup", ...RUNS_WITH },
      { hook_event_name: "UserPromptSubmit", prompt: "Tidy up and tell me all about it", ...RUNS_WITH },
      { hook_event_name: "Stop", last_assistant_message: LONG_ANSWER, ...RUNS_WITH },
    ]),

  // A second session that asks in its own terminal (no card here): its pill
  // gets the amber ring and "Needs approval", a notice says it waits, and
  // nothing can be answered from the island.
  async waiting() {
    const [a, b] = [session("a", join(cwd, "src")), session("b", join(cwd, "docs"))];
    await play([{ hook_event_name: "UserPromptSubmit", prompt: "Work here" }, ...tool("Read", { file_path: join(a.cwd, "main.ts") })], a);
    await play([{ hook_event_name: "UserPromptSubmit", prompt: "Work there" }], b);
    await play([{ hook_event_name: "Notification", notification_type: "permission_prompt", message: "Claude needs your permission to use Bash" }], b);
    console.log("B waits in its terminal for fifteen seconds: amber ring on its pill, one notice, no card.");
    await sleep(15000);
    await play([...tool("Bash", { command: "ls" }).slice(1), { hook_event_name: "Stop", last_assistant_message: "Done there." }], b);
    await play([{ hook_event_name: "Stop", last_assistant_message: "Done here." }], a, 6000);
    await play([{ hook_event_name: "SessionEnd" }], b);
    await play([{ hook_event_name: "SessionEnd" }], a);
  },

  // Do not click anything. The relay must come back by itself after its time
  // limit with nothing printed: nobody answered, so nothing is answered.
  async timeout() {
    console.log("One permission request. Do not answer it: the relay must give up by itself (about 110 s).");
    const event = permission("rm -rf ./nobody-answers  # fake");
    report(event, await send(event));
  },
};

const scene = process.argv[2] ?? "session";
if (!SCENES[scene]) {
  console.error(`Unknown scene "${scene}". One of: ${Object.keys(SCENES).join(", ")}.`);
  process.exit(1);
}
console.log(`relay: ${hook}\nscene: ${scene}\n`);
await SCENES[scene]();

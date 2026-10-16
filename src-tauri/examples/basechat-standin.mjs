// A stand-in for every program the chat starts (an agent CLI, or gh), so that
// nothing real runs, and nobody's plan is spent, while the chat is worked on.
// It prints a canned answer in the format of the CLI its arguments belong to,
// and does nothing else.
//
// A debug build of the app starts it in place of all of them when
// METEOR_CLI_STAND_IN holds this file's full path (src-tauri/src/chat/program.rs):
//
//   $env:METEOR_CLI_STAND_IN = "$PWD\src-tauri\examples\basechat-standin.mjs"; npm run tauri dev
//
// The answer says how the message arrived, which is what the probe checks. A
// message containing STANDIN-SLOW makes it hang (to try Stop, Cancel and the
// time limits), STANDIN-BIG makes it print 2 MB (the size cap), and
// STANDIN-FAIL makes it fail the way a signed-out CLI does.

const args = process.argv.slice(2);
const has = (arg) => args.includes(arg);

let input = "";
for await (const chunk of process.stdin) input += chunk;

// What the user's words arrived as: everything after `--`, else the input.
// Several arguments after `--` would mean the message was split on its way.
const dash = args.indexOf("--");
const asked = dash >= 0 ? args.slice(dash + 1).join("\u0000") : input;
const how = dash >= 0 ? "one argument after --" : "the input";

if (asked.includes("STANDIN-SLOW")) await new Promise((resolve) => setTimeout(resolve, 120_000));
if (asked.includes("STANDIN-BIG")) process.stdout.write(`${"x".repeat(2_000_000)}\n`);
if (asked.includes("STANDIN-FAIL")) {
  console.error("Error: not logged in. Please run /login");
  process.exit(1);
}

const answer = `**Stand-in answer.** ${args.length} arguments, the message came as ${how}:\n\n${asked}`;
const say = (event) => console.log(JSON.stringify(event));

if (args[0] === "auth" && args[1] === "token") {
  // gh auth token
  console.log("gho_standin000000000000000000000000000000");
} else if (has("--append-system-prompt")) {
  // claude -p --output-format json: a task.
  say({ type: "result", subtype: "success", is_error: false, result: `## Done\n\n${answer}` });
} else if (has("--include-partial-messages")) {
  // claude -p --output-format stream-json: a chat turn, in three pieces.
  say({ type: "system", subtype: "init", session_id: "00000000-0000-4000-8000-000000000000" });
  const third = Math.ceil(answer.length / 3);
  for (let at = 0; at < answer.length; at += third) {
    say({ type: "stream_event", event: { type: "content_block_delta", delta: { type: "text_delta", text: answer.slice(at, at + third) } } });
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  say({ type: "result", subtype: "success", is_error: false, result: answer });
} else if (args[0] === "exec") {
  // codex exec --json
  say({ type: "thread.started", thread_id: "stand-in" });
  say({ type: "item.completed", item: { type: "agent_message", text: answer } });
} else if (has("--approval-mode")) {
  // gemini -p … -o stream-json
  say({ type: "init", session_id: "stand-in" });
  say({ type: "message", role: "assistant", content: answer });
  say({ type: "result", status: "success" });
} else if (has("--mode")) {
  // cursor-agent -p --output-format json --mode ask
  say({ type: "result", is_error: false, result: answer, session_id: "stand-in" });
} else if (args[0] === "chat") {
  // kiro-cli chat --no-interactive: plain text after a coloured mark.
  console.log(`\u001b[32m> \u001b[0m${answer}`);
} else {
  console.error(`stand-in: no CLI takes these arguments: ${JSON.stringify(args)}`);
  process.exit(2);
}

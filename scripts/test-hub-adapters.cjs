// node scripts/test-hub-adapters.cjs   (needs Playwright: `npm i -D playwright` or a global install)
// Runs each adapter against a hand-made page that follows ITS OWN selectors. This tests the adapter's logic
// (state, busy, send, tools); it says nothing about whether the real sites still use these selectors.
const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');
const adapters = path.join(__dirname, '..', 'crates', 'hub', 'adapters');
const mocks = {
  claude: `<div contenteditable="true" role="textbox" id="in"></div><button aria-label="Send message" id="send">send</button>
    <button aria-label="Search and tools" onclick="document.body.insertAdjacentHTML('beforeend','<div role=menuitemcheckbox aria-checked=true><div>Supabase</div><small>connected</small></div><div role=menuitemcheckbox aria-checked=false>Notion</div><div role=menuitemcheckbox aria-checked=true>Vercel</div>')">t</button>
    <div data-testid="assistant-message">Hello there</div>`,
  chatgpt: `<div id="prompt-textarea" contenteditable="true"></div><button data-testid="send-button" id="send">send</button>
    <button data-testid="composer-plus-btn" onclick="document.body.insertAdjacentHTML('beforeend','<div role=menuitem>Web search</div><div role=menuitem>Deep research</div><div role=menuitem>Add photos</div><div role=menuitemcheckbox aria-checked=true>GitHub</div>')">+</button>
    <div data-message-author-role="assistant">Hi from GPT</div>`,
  gemini: `<rich-textarea><div class="ql-editor" contenteditable="true"></div></rich-textarea><button aria-label="Send message" id="send">send</button>
    <button aria-label="Tools" onclick="document.body.insertAdjacentHTML('beforeend','<button role=menuitem>Deep Research</button><button role=menuitem>Canvas</button><button role=menuitem>Create images</button>')">tools</button>
    <model-response><message-content>Gemini says hi</message-content></model-response>`,
  grok: `<form><textarea aria-label="Ask Grok anything"></textarea><button type="submit" id="send">go</button><button>DeepSearch</button><button>Think</button><button>Attach</button></form>
    <div class="message-bubble">Grok here</div>`,
};
(async () => {
  const b = await chromium.launch();
  let bad = 0;
  const check = (id, name, ok, extra = '') => { console.log(ok ? 'PASS' : 'FAIL', id, name, extra); if (!ok) bad++; };
  for (const [id, html] of Object.entries(mocks)) {
    const p = await b.newPage();
    await p.setContent(`<html><body>${html}</body></html>`);
    await p.evaluate(() => { window.sent = 0; document.getElementById('send').addEventListener('click', (e) => { e.preventDefault(); window.sent++; }); });
    await p.evaluate(fs.readFileSync(path.join(adapters, `${id}.js`), 'utf8'));
    const st = await p.evaluate(() => window.__hubAdapter.state());
    check(id, 'state ready + last reply', st.ready && st.last.length > 0 && st.busy === false, JSON.stringify(st));
    const sent = await p.evaluate(() => window.__hubAdapter.send('Task from claude via the hub:\nline two "quoted"'));
    await p.waitForTimeout(600);
    const typed = await p.evaluate(() => { const i = document.querySelector('[contenteditable=true],textarea'); return i.value ?? i.innerText; });
    const clicks = await p.evaluate(() => window.sent);
    check(id, 'send types the text', sent.ok && typed.includes('line two "quoted"'), JSON.stringify(typed));
    check(id, 'send presses send once', clicks === 1, String(clicks));
    // busy: a stop button appears
    const stopSel = { claude: 'aria-label="Stop response"', chatgpt: 'data-testid="stop-button"', gemini: 'aria-label="Stop response"', grok: 'aria-label="Stop model response"' }[id];
    await p.evaluate((s) => document.body.insertAdjacentHTML('beforeend', `<button ${s}>stop</button>`), stopSel);
    check(id, 'busy while a stop button shows', (await p.evaluate(() => window.__hubAdapter.state())).busy === true);
    const open = await p.evaluate(() => window.__hubAdapter.toolsOpen());
    if (id === 'grok') check(id, 'toolsOpen is a no-op', open.ok === true);
    else {
      check(id, 'toolsOpen finds the button', open.ok === true);
      // the buttons' onclick adds the menu only when the page's own button is clicked: the adapter clicks it.
    }
    await p.waitForTimeout(200);
    const tools = await p.evaluate(() => window.__hubAdapter.toolsRead());
    console.log('     tools ->', JSON.stringify(tools.tools));
    const want = { claude: ['supabase', 'vercel'], chatgpt: ['browse', 'research', 'github'], gemini: ['research', 'browse', 'canvas', 'image'], grok: ['research', 'browse', 'reasoning'] }[id];
    check(id, 'tools mapped', want.every((w) => tools.tools.includes(w)) && !tools.tools.includes('notion'));
    await p.close();
  }
  await b.close();
  console.log(bad ? `${bad} FAILED` : 'ALL PASSED');
  process.exitCode = bad ? 1 : 0;
})();

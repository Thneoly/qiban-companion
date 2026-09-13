// Engineering checks only. These scripted interactions are NOT research data.
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const html = fs.readFileSync(require('node:path').resolve(__dirname, '../../prototypes/companion-validation/index.html'), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
new vm.Script(script);
function environment() {
  const elements = new Map();
  const handlers = {};
  let time = 0;
  const document = {
    hidden: false,
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, { value: '', innerHTML: '', querySelectorAll: () => [] });
      return elements.get(id);
    },
    addEventListener(name, handler) { handlers[name] = handler; }
  };
  const context = vm.createContext({
    document, window: { scrollTo() {} }, crypto: { randomUUID: () => 'engineering-check' },
    performance: { now: () => time }, Date, Math, Blob, URL, setTimeout,
    FormData: class { constructor(form) { return Object.entries(form.answers); } }
  });
  vm.runInContext(script, context);
  return { document, handlers, run: code => vm.runInContext(code, context), tick: ms => time += ms };
}
let runs = 0;
for (const quality of ['accurate', 'omission']) {
  for (let sequence = 0; sequence < 4; sequence++) {
    const e = environment();
    const el = id => e.document.getElementById(id);
    e.run(`state.summaryQuality='${quality}'`);
    el('sequence').value = String(sequence);
    el('start').onclick();
    for (let round = 0; round < 2; round++) {
      const mode = e.run('state.live.mode');
      if (mode === 'summary') {
        const text = e.run('state.summaryQuality===\'accurate\'?accurateSummaries[state.live.scenario]:scenarios[state.live.scenario].summary');
        assert.ok(el('app').innerHTML.includes(text));
      }
      const answers = e.run('Object.fromEntries(scenarios[state.live.scenario].tasks.map(t=>[t.id,t.correct]))');
      e.tick(1000);
      e.document.hidden = true;
      e.handlers.visibilitychange();
      e.tick(2000);
      e.document.hidden = false;
      e.handlers.visibilitychange();
      e.tick(3000);
      el('decisions').onsubmit({ preventDefault() {}, target: { answers } });
      el('effort').value = '0';
      el('difficulty').value = '<script>test only</script>';
      el('rating').onsubmit({ preventDefault() {} });
    }
    const data = JSON.parse(e.run('JSON.stringify(dataExport())'));
    assert.equal(data.endedReason, 'completed');
    assert.equal(data.summaryQuality, quality);
    assert.equal(data.trials.length, 2);
    assert.equal(new Set(data.trials.map(t => t.mode)).size, 2);
    for (const t of data.trials) {
      assert.equal(t.score.correct, 3);
      assert.equal(t.score.criticalErrors, 0);
      assert.equal(t.wallMs, 6000);
      assert.equal(t.visibleMs, 4000);
      assert.equal(t.hiddenCount, 1);
      assert.equal(t.effort, 0); // zero must not be treated as missing
      assert.equal(t.start, undefined);
    }
    assert.ok(el('app').innerHTML.includes('试测结束'));
    assert.ok(!el('app').innerHTML.includes('<script>test only</script>'));
    el('preview').onclick();
    el('toggle-role').onclick();
    el('back-results').onclick();
    assert.equal(e.run('state.preview.length'), 1);
    runs++;
  }
}
const e = environment();
e.run('state.sequence=0;renderTrial()');
e.document.getElementById('exit').onclick();
assert.equal(e.run('state.trials[0].status'), 'abandoned');
assert.equal(e.run('score(state.trials[0])'), null);
assert.equal(e.run('state.endedReason'), 'participant-exit');
const wrong = JSON.parse(e.run(`JSON.stringify(score({scenario:'launch',answers:{page:'publish',mail:'send',outline:'approve'}}))`));
assert.equal(wrong.unsafeActions, 2);
assert.equal(wrong.unnecessaryStops, 1);
assert.equal(wrong.correct, 0);
assert.ok(!/\b(fetch|XMLHttpRequest|WebSocket|localStorage|sessionStorage)\b/.test(script));
console.log(`PASS: ${runs} scripted two-round flows; both summary versions; timing, scoring, exit, export payload and role toggle checks.`);
console.log('DOM is stubbed. No real-browser visual, native form validation or download checks. No participant data generated.');

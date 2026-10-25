// Dependency-free render test, using the actual Python catalog and shared policy.
// node scripts/test-station-ui.cjs ../pipeline-workflows
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const {spawnSync} = require('node:child_process');
const assert = require('node:assert/strict');
const root = path.resolve(process.argv[2] || '../pipeline-workflows');
const python = process.env.FORGE_PYTHON || 'python';
const run = spawnSync(python, [path.join(root, 'API/_system/engine/console.py'), 'list'], {encoding:'utf8'});
assert.equal(run.status, 0, run.stderr);
const feed = JSON.parse(run.stdout);
const policy = JSON.parse(fs.readFileSync(path.join(root, 'API/_system/config/workspace.json'), 'utf8'));
const html = fs.readFileSync(path.join(__dirname, '../ui/home.html'), 'utf8');
const source = html.slice(html.indexOf('function renderStations(){'), html.indexOf('\nasync function load(){'));
const elements = {'#q': {value:''}, '#stations': {innerHTML:''}};
const context = {
  all: feed.stations,
  EXCLUDED_FROM_PAGE: policy.EXCLUDED_FROM_PAGE,
  stationGroups: policy.groups,
  $: selector => elements[selector],
  esc: value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]))
};
vm.runInNewContext(source + '\nrenderStations();', context);
const visible = feed.stations.filter(s => !policy.EXCLUDED_FROM_PAGE.includes(s.label));
assert.equal((elements['#stations'].innerHTML.match(/class="card"/g) || []).length, visible.length);
for (const label of ['41_STORY', '042_KAIT_STORIES', '101_FIS_INVENTORY']) {
  assert.ok(elements['#stations'].innerHTML.includes(label), label + ' must be visible');
}
assert.ok(elements['#stations'].innerHTML.includes('Front stations'));
elements['#q'].value = 'KAIT';
vm.runInNewContext(source + '\nrenderStations();', context);
assert.equal((elements['#stations'].innerHTML.match(/class="card"/g) || []).length, 1);
for (const file of ['home.html', 'api-workspace.html']) {
  const text = fs.readFileSync(path.join(__dirname, '../ui', file), 'utf8');
  for (const match of text.matchAll(/<script>([\s\S]*?)<\/script>/g)) new vm.Script(match[1]);
}
console.log(`${visible.length} visible stations render; Stories, Kait, FIS and filtering pass; both pages parse.`);

import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {test} from 'node:test';

const generator = fileURLToPath(new URL('../gen_tests.js', import.meta.url));

function fixture(t, exclusions) {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'fluent-fixture-generator-'));
    t.after(() => fs.rmSync(root, {recursive: true, force: true}));
    const write = (name, value) => fs.writeFileSync(path.join(root, name), JSON.stringify(value));
    fs.mkdirSync(path.join(root, 'src'));
    fs.mkdirSync(path.join(root, 'fixtures/state_tests/for_osaka'), {recursive: true});
    fs.copyFileSync(generator, path.join(root, 'gen_tests.js'));
    write('package.json', {type: 'module'});
    write('ethereum-tests.json', {release: 'test', directory: 'fixtures', forks: ['Osaka']});
    write('ci-tests.json', [{path: 'active.json', forks: ['Osaka']}]);
    write('excluded-tests.json', exclusions);
    for (const name of ['active', 'excluded']) {
        write(`fixtures/state_tests/for_osaka/${name}.json`, {test: {post: {Osaka: [{}]}}});
    }
    return {root, run: (...args) => spawnSync(process.execPath, [path.join(root, 'gen_tests.js'), ...args], {encoding: 'utf8'})};
}

const exclusion = {fork: 'Osaka', path: 'excluded.json', reason: 'Documented protocol difference', source: 'protocol specification'};

test('preserves exclusion reasons and detects removed annotations', t => {
    const {root, run} = fixture(t, [exclusion]);
    assert.equal(run().status, 0);
    const output = path.join(root, 'src/tests.rs');
    assert.match(fs.readFileSync(output, 'utf8'), /#\[ignore = "Documented protocol difference"\]/);
    assert.equal(run('--check').status, 0);
    fs.writeFileSync(output, fs.readFileSync(output, 'utf8').replace(/\s*#\[ignore[^\n]+\n/, '\n'));
    assert.notEqual(run('--check').status, 0);
});

test('rejects exclusions without reasons', t => {
    const {run} = fixture(t, [{...exclusion, reason: ''}]);
    assert.match(run().stderr, /reason and source/);
});

test('rejects excluding the CI selection', t => {
    const {run} = fixture(t, [{...exclusion, path: 'active.json'}]);
    assert.match(run().stderr, /CI fixture is excluded/);
});

test('rejects stale exclusion paths', t => {
    const {run} = fixture(t, [{...exclusion, path: 'missing.json'}]);
    assert.match(run().stderr, /Stale exclusions/);
});

test('partial exclusions keep the fixture enabled and count only matching posts', t => {
    const {root, run} = fixture(t, [{...exclusion, cases: ['mixed'], exceptions: ['FloorGas']}]);
    fs.writeFileSync(path.join(root, 'fixtures/state_tests/for_osaka/excluded.json'), JSON.stringify({
        mixed: {post: {Osaka: [{expectException: 'IntrinsicGas|FloorGas'}, {}, {expectException: 'OtherError'}]}},
        supported: {post: {Osaka: [{expectException: 'FloorGas'}]}}
    }));
    const result = run();
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /1 explicitly excluded \(0 whole files, 1 partial files\)/);
    assert.doesNotMatch(fs.readFileSync(path.join(root, 'src/tests.rs'), 'utf8'), /#\[ignore/);
});

test('rejects stale case and exception selectors', t => {
    for (const selector of [{cases: ['missing']}, {exceptions: ['MissingError']}]) {
        const {run} = fixture(t, [{...exclusion, ...selector}]);
        assert.match(run().stderr, /Stale exclusion selector/);
    }
});

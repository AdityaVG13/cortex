'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const net = require('node:net');

const REPO_ROOT = path.resolve(__dirname, '..');
const SRC_ROOT = path.join(REPO_ROOT, 'src');
const PROFILES_PATH = path.join(REPO_ROOT, 'cortex-profiles.json');

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function toSqliteDate(date) {
  return date.toISOString().slice(0, 19).replace('T', ' ');
}

async function getFreePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      server.close((err) => (err ? reject(err) : resolve(port)));
    });
    server.on('error', reject);
  });
}

function replacePortConstant(filePath, original, next) {
  const content = fs.readFileSync(filePath, 'utf-8');
  fs.writeFileSync(filePath, content.replace(original, next), 'utf-8');
}

function appendCliTestExports(filePath) {
  const content = fs.readFileSync(filePath, 'utf-8');
  const patched = content.replace(
    '\nmain();\n',
    '\nif (!process.env.CORTEX_TEST_NO_MAIN) {\n  main();\n}\n\nmodule.exports = {\n  _test: {\n    request,\n    cmdBoot,\n    cmdRecall,\n    cmdStore,\n    cmdHealth,\n    cmdStatus,\n    ensureDaemon,\n    isDaemonAlive,\n  },\n};\n'
  );
  fs.writeFileSync(filePath, patched, 'utf-8');
}

async function createSandbox() {
  const port = await getFreePort();
  const root = fs.mkdtempSync(path.join(REPO_ROOT, '.tmp-test-'));
  const srcDir = path.join(root, 'src');
  const homeDir = path.join(root, 'home');

  fs.mkdirSync(srcDir, { recursive: true });
  fs.mkdirSync(homeDir, { recursive: true });
  fs.cpSync(SRC_ROOT, srcDir, { recursive: true });
  fs.copyFileSync(PROFILES_PATH, path.join(root, 'cortex-profiles.json'));

  replacePortConstant(path.join(srcDir, 'daemon.js'), 'const PORT = 7437;', `const PORT = ${port};`);
  replacePortConstant(path.join(srcDir, 'cli.js'), 'const DAEMON_PORT = 7437;', `const DAEMON_PORT = ${port};`);
  appendCliTestExports(path.join(srcDir, 'cli.js'));

  return {
    root,
    srcDir,
    homeDir,
    port,
    cleanup() {
      fs.rmSync(root, { recursive: true, force: true });
    },
  };
}

function withSandboxEnv(homeDir, fn) {
  const prevHome = process.env.HOME;
  const prevUserProfile = process.env.USERPROFILE;
  process.env.HOME = homeDir;
  process.env.USERPROFILE = homeDir;
  try {
    return fn();
  } finally {
    if (prevHome === undefined) delete process.env.HOME;
    else process.env.HOME = prevHome;
    if (prevUserProfile === undefined) delete process.env.USERPROFILE;
    else process.env.USERPROFILE = prevUserProfile;
  }
}

function loadSandboxModule(sandbox, relPath) {
  const absPath = path.join(sandbox.srcDir, relPath);
  delete require.cache[require.resolve(absPath)];
  return withSandboxEnv(sandbox.homeDir, () => require(absPath));
}

function vectorForPrompt(prompt, dim) {
  const lower = String(prompt).toLowerCase();
  const vec = new Float32Array(dim);
  if (lower.includes('uv') || lower.includes('python installs') || lower.includes('same-vector')) {
    vec[0] = 1;
    return vec;
  }
  if (lower.includes('orthogonal')) {
    vec[1] = 1;
    return vec;
  }
  vec[2] = 1;
  return vec;
}

function installFakeOllama(dim) {
  const originalFetch = global.fetch;
  global.fetch = async (url, options = {}) => {
    const target = String(url);
    if (target.endsWith('/api/embeddings')) {
      const body = JSON.parse(options.body || '{}');
      return {
        ok: true,
        status: 200,
        statusText: 'OK',
        json: async () => ({ embedding: Array.from(vectorForPrompt(body.prompt || '', dim)) }),
      };
    }

    if (target.endsWith('/api/tags')) {
      return {
        ok: true,
        status: 200,
        statusText: 'OK',
        json: async () => ({ models: [{ name: 'nomic-embed-text' }] }),
      };
    }

    throw new Error(`Unexpected fetch target in test: ${target}`);
  };

  return () => {
    global.fetch = originalFetch;
  };
}

function httpJson(port, method, pathname, { headers = {}, body } = {}) {
  return new Promise((resolve, reject) => {
    const payload = body ? JSON.stringify(body) : null;
    const req = http.request(
      {
        host: '127.0.0.1',
        port,
        path: pathname,
        method,
        headers: {
          Host: `127.0.0.1:${port}`,
          ...(payload ? { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) } : {}),
          ...headers,
        },
      },
      (res) => {
        let data = '';
        res.on('data', (chunk) => { data += chunk; });
        res.on('end', () => {
          let json = null;
          try {
            json = data ? JSON.parse(data) : null;
          } catch {
            json = null;
          }
          resolve({ statusCode: res.statusCode, body: json, raw: data });
        });
      }
    );

    req.on('error', reject);
    if (payload) req.write(payload);
    req.end();
  });
}

async function waitForHealth(port, timeoutMs = 4000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = null;

  while (Date.now() < deadline) {
    try {
      const res = await httpJson(port, 'GET', '/health');
      if (res.statusCode === 200 && res.body?.status === 'ok') {
        return res.body;
      }
    } catch (err) {
      lastError = err;
    }
    await sleep(50);
  }

  throw lastError || new Error(`Timed out waiting for daemon on port ${port}`);
}

async function startDaemonInProcess(sandbox) {
  const daemonPath = path.join(sandbox.srcDir, 'daemon.js');
  const originalArgv = process.argv.slice();
  const originalExit = process.exit;
  const originalStderrWrite = process.stderr.write;

  process.argv = ['node', daemonPath, 'serve'];
  process.exit = () => {};

  withSandboxEnv(sandbox.homeDir, () => {
    delete require.cache[require.resolve(daemonPath)];
    require(daemonPath);
  });

  process.argv = originalArgv;
  await waitForHealth(sandbox.port);

  return {
    async stop(token) {
      try {
        await httpJson(sandbox.port, 'POST', '/shutdown', {
          headers: { Authorization: `Bearer ${token}` },
        });
        await sleep(100);
      } catch {
        // Daemon may already be closed.
      } finally {
        process.exit = originalExit;
        process.stderr.write = originalStderrWrite;
      }
    },
  };
}

async function runCliCommandInProcess(sandbox, args) {
  const cliPath = path.join(sandbox.srcDir, 'cli.js');
  const originalArgv = process.argv.slice();
  const originalStdout = process.stdout.write;
  const originalStderr = process.stderr.write;
  const originalExitCode = process.exitCode;

  let stdout = '';
  let stderr = '';

  process.argv = ['node', cliPath, ...args];
  process.env.CORTEX_TEST_NO_MAIN = '1';
  process.stdout.write = (chunk, encoding, callback) => {
    stdout += String(chunk);
    if (typeof callback === 'function') callback();
    return true;
  };
  process.stderr.write = (chunk, encoding, callback) => {
    stderr += String(chunk);
    if (typeof callback === 'function') callback();
    return true;
  };

  let code = 0;

  try {
    const cli = loadSandboxModule(sandbox, 'cli.js')._test;
    let exitCode;
    switch (args[0]) {
      case 'health':
        exitCode = await cli.cmdHealth();
        break;
      case 'store':
        exitCode = await cli.cmdStore([args[1]], { agent: args[3] });
        break;
      case 'recall':
        exitCode = await cli.cmdRecall([args[1]], {});
        break;
      default:
        throw new Error(`Unsupported in-process CLI command: ${args[0]}`);
    }
    code = exitCode ?? process.exitCode ?? 0;
  } finally {
    process.argv = originalArgv;
    process.stdout.write = originalStdout;
    process.stderr.write = originalStderr;
    process.exitCode = originalExitCode;
    delete process.env.CORTEX_TEST_NO_MAIN;
  }

  return { code, stdout, stderr };
}

test('semantic conflict detection works with real embedding blobs', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const db = loadSandboxModule(sandbox, 'db.js');
  const embeddings = loadSandboxModule(sandbox, 'embeddings.js');
  const conflict = loadSandboxModule(sandbox, 'conflict.js');
  const restoreFetch = installFakeOllama(embeddings.EMBED_DIM);
  t.after(() => restoreFetch());

  await withSandboxEnv(sandbox.homeDir, () => db.getDb());
  const id = db.insert(
    'INSERT INTO decisions (decision, context, type, source_agent, confidence, surprise) VALUES (?, ?, ?, ?, ?, ?)',
    ['Use uv for python installs', 'python-env', 'decision', 'agent-a', 0.9, 0.9]
  );
  db.insert(
    'INSERT INTO embeddings (target_type, target_id, vector, model) VALUES (?, ?, ?, ?)',
    ['decision', id, embeddings.vectorToBlob(vectorForPrompt('same-vector', embeddings.EMBED_DIM)), embeddings.EMBED_MODEL]
  );

  const result = await conflict.detectConflict('same-vector decision text', 'agent-b');
  assert.equal(result.isConflict, true);
  assert.equal(result.matchedId, id);
  assert.equal(result.matchedAgent, 'agent-a');
  assert.ok(result.similarity > 0.85);

  db.close();
});

test('brain happy path covers store, recall, diary write, and health stats', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const db = loadSandboxModule(sandbox, 'db.js');
  const embeddings = loadSandboxModule(sandbox, 'embeddings.js');
  const brain = loadSandboxModule(sandbox, 'brain.js');
  const restoreFetch = installFakeOllama(embeddings.EMBED_DIM);
  t.after(() => restoreFetch());

  await withSandboxEnv(sandbox.homeDir, () => db.getDb());

  const stored = await brain.store('Use uv for python installs', {
    context: 'python-env',
    source_agent: 'agent-a',
  });
  assert.equal(stored.stored, true);
  assert.equal(stored.status, 'active');

  await embeddings.buildEmbeddings();

  const results = await brain.recall('uv python installs');
  assert.ok(results.length > 0);
  assert.match(results[0].excerpt, /Use uv for python installs/);

  const storedDecision = db.get('SELECT retrievals, last_accessed FROM decisions WHERE id = ?', [stored.id]);
  assert.equal(storedDecision.retrievals, 1);
  assert.ok(storedDecision.last_accessed);

  const diaryResult = brain.writeDiary({
    accomplished: 'Added the happy path test',
    nextSteps: 'Keep building Cortex',
    decisions: 'Use uv for python installs',
    pending: 'Add more coverage',
    knownIssues: 'None',
  });
  assert.equal(diaryResult.written, true);

  const statePath = path.join(sandbox.homeDir, '.claude', 'state.md');
  const stateText = fs.readFileSync(statePath, 'utf-8');
  assert.match(stateText, /## Key Decisions/);
  assert.match(stateText, /Use uv for python installs/);

  const stats = await brain.getStats();
  assert.equal(stats.ollama, 'connected');
  assert.equal(stats.decisions, 1);
  assert.ok(stats.embeddings >= 1);

  db.close();
});

test('keyword fallback uses tokenized OR matching and ranks by matches, recency, and score', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const db = loadSandboxModule(sandbox, 'db.js');
  await withSandboxEnv(sandbox.homeDir, () => db.getDb());

  const now = new Date();
  const old = new Date(now.getTime() - (30 * 24 * 60 * 60 * 1000));

  db.insert(
    'INSERT INTO memories (text, source, type, score, created_at, updated_at, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?)',
    ['alpha beta guidance', 'memory::alpha-beta', 'memory', 0.8, toSqliteDate(now), toSqliteDate(now), toSqliteDate(now)]
  );
  db.insert(
    'INSERT INTO memories (text, source, type, score, created_at, updated_at, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?)',
    ['alpha note only', 'memory::alpha-only', 'memory', 5.0, toSqliteDate(now), toSqliteDate(now), toSqliteDate(now)]
  );
  db.insert(
    'INSERT INTO memories (text, source, type, score, created_at, updated_at, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?)',
    ['beta archive note', 'memory::beta-old', 'memory', 5.0, toSqliteDate(old), toSqliteDate(old), toSqliteDate(old)]
  );

  const rankedMemories = db.searchMemories('alpha beta');
  assert.equal(rankedMemories.length, 3);
  assert.equal(rankedMemories[0].source, 'memory::alpha-beta');
  assert.equal(rankedMemories[1].source, 'memory::alpha-only');
  assert.equal(rankedMemories[2].source, 'memory::beta-old');
  assert.equal(rankedMemories[0]._matched_keywords, 2);
  assert.equal(rankedMemories[1]._matched_keywords, 1);

  db.insert(
    'INSERT INTO decisions (decision, context, type, score, created_at, updated_at, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?)',
    ['beta rollout plan', 'decision::beta', 'decision', 0.6, toSqliteDate(now), toSqliteDate(now), toSqliteDate(now)]
  );
  db.insert(
    'INSERT INTO decisions (decision, context, type, score, created_at, updated_at, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?)',
    ['alpha beta canonical rule', 'decision::alpha-beta', 'decision', 0.7, toSqliteDate(now), toSqliteDate(now), toSqliteDate(now)]
  );

  const rankedDecisions = db.searchDecisions('alpha beta');
  assert.equal(rankedDecisions.length, 2);
  assert.equal(rankedDecisions[0].context, 'decision::alpha-beta');
  assert.equal(rankedDecisions[1].context, 'decision::beta');

  db.close();
});

test('brain init runs decay pass and pinned memories do not decay', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const db = loadSandboxModule(sandbox, 'db.js');
  const embeddings = loadSandboxModule(sandbox, 'embeddings.js');
  const restoreFetch = installFakeOllama(embeddings.EMBED_DIM);
  t.after(() => restoreFetch());

  await withSandboxEnv(sandbox.homeDir, () => db.getDb());

  const staleDate = new Date(Date.now() - (10 * 24 * 60 * 60 * 1000));
  const staleText = toSqliteDate(staleDate);

  const staleMemoryId = db.insert(
    'INSERT INTO memories (text, source, type, score, pinned, created_at, updated_at, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?, ?)',
    ['stale memory', 'memory::stale', 'memory', 1.0, 0, staleText, staleText, staleText]
  );
  const pinnedMemoryId = db.insert(
    'INSERT INTO memories (text, source, type, score, pinned, created_at, updated_at, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?, ?)',
    ['pinned memory', 'memory::pinned', 'memory', 1.0, 1, staleText, staleText, staleText]
  );
  const staleDecisionId = db.insert(
    'INSERT INTO decisions (decision, context, type, score, pinned, created_at, updated_at, last_accessed) VALUES (?, ?, ?, ?, ?, ?, ?, ?)',
    ['stale decision', 'decision::stale', 'decision', 1.0, 0, staleText, staleText, staleText]
  );

  db.close();

  const brain = loadSandboxModule(sandbox, 'brain.js');
  await brain.init();

  const reopenedDb = loadSandboxModule(sandbox, 'db.js');
  await withSandboxEnv(sandbox.homeDir, () => reopenedDb.getDb());

  const staleMemory = reopenedDb.get('SELECT score, updated_at FROM memories WHERE id = ?', [staleMemoryId]);
  const pinnedMemory = reopenedDb.get('SELECT score FROM memories WHERE id = ?', [pinnedMemoryId]);
  const staleDecision = reopenedDb.get('SELECT score FROM decisions WHERE id = ?', [staleDecisionId]);

  assert.ok(staleMemory.score < 1.0);
  assert.ok(staleMemory.score >= 0.59 && staleMemory.score <= 0.6);
  assert.equal(pinnedMemory.score, 1.0);
  assert.ok(staleDecision.score < 1.0);

  reopenedDb.close();
});

test('daemon startup, auth protections, and CLI critical path stay fixed', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const health = await httpJson(sandbox.port, 'GET', '/health');
  assert.equal(health.statusCode, 200);
  assert.equal(health.body.status, 'ok');
  assert.ok(health.body.stats);

  for (const pathname of ['/diary', '/forget', '/resolve']) {
    const res = await httpJson(sandbox.port, 'POST', pathname, { body: {} });
    assert.equal(res.statusCode, 401, `${pathname} should require auth`);
  }

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const diary = await httpJson(sandbox.port, 'POST', '/diary', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      accomplished: 'Verified auth',
      nextSteps: 'Add regression tests',
      decisions: 'Preserve key decisions through the API',
      pending: 'Keep the daemon stable',
      knownIssues: 'None',
    },
  });
  assert.equal(diary.statusCode, 200);
  assert.equal(diary.body.written, true);

  const statePath = path.join(sandbox.homeDir, '.claude', 'state.md');
  const stateText = fs.readFileSync(statePath, 'utf-8');
  assert.match(stateText, /## Key Decisions/);
  assert.match(stateText, /Preserve key decisions through the API/);

  const cliHealth = await runCliCommandInProcess(sandbox, ['health']);
  assert.equal(cliHealth.code, 0, cliHealth.stderr);
  assert.match(cliHealth.stdout, /Status:\s+ok/);
  assert.doesNotMatch(cliHealth.stdout, /\?/);

  const cliStore = await runCliCommandInProcess(sandbox, ['store', 'Use uv for python installs', '--agent', 'tester']);
  assert.equal(cliStore.code, 0, cliStore.stderr);
  assert.match(cliStore.stdout, /Stored: Use uv for python installs/);
  assert.match(cliStore.stdout, /Surprise:/);

  const cliRecall = await runCliCommandInProcess(sandbox, ['recall', 'uv']);
  assert.equal(cliRecall.code, 0, cliRecall.stderr);
  assert.match(cliRecall.stdout, /Use uv for python installs/);
});

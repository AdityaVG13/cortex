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
          resolve({ statusCode: res.statusCode, headers: res.headers, body: json, raw: data });
        });
      }
    );

    req.on('error', reject);
    if (payload) req.write(payload);
    req.end();
  });
}

function readFirstSseEvent(port, pathname = '/events/stream', timeoutMs = 3000) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        host: '127.0.0.1',
        port,
        path: pathname,
        method: 'GET',
        headers: {
          Host: `127.0.0.1:${port}`,
        },
      },
      (res) => {
        let buffer = '';

        const timeout = setTimeout(() => {
          req.destroy(new Error(`Timed out waiting for SSE frame from ${pathname}`));
        }, timeoutMs);

        function cleanup() {
          clearTimeout(timeout);
          res.removeAllListeners('data');
          res.removeAllListeners('end');
          res.removeAllListeners('error');
        }

        res.on('data', (chunk) => {
          buffer += String(chunk);
          const frameEnd = buffer.indexOf('\n\n');
          if (frameEnd === -1) return;

          const frame = buffer.slice(0, frameEnd);
          const lines = frame.split('\n');
          const event = lines.find((line) => line.startsWith('event:'))?.slice('event:'.length).trim() || null;
          const dataRaw = lines.find((line) => line.startsWith('data:'))?.slice('data:'.length).trim() || null;

          let data = null;
          if (dataRaw) {
            try {
              data = JSON.parse(dataRaw);
            } catch {
              data = dataRaw;
            }
          }

          cleanup();
          req.destroy();
          resolve({
            statusCode: res.statusCode,
            headers: res.headers,
            event,
            data,
            raw: frame,
          });
        });

        res.on('end', () => {
          cleanup();
          reject(new Error(`SSE stream ended before first frame on ${pathname}`));
        });

        res.on('error', (err) => {
          cleanup();
          reject(err);
        });
      }
    );

    req.on('error', reject);
    req.end();
  });
}

function assertRecallResultContract(result) {
  assert.equal(typeof result.source, 'string');
  assert.ok(result.source.length > 0);
  assert.equal(typeof result.relevance, 'number');
  assert.ok(Number.isFinite(result.relevance));
  assert.equal(typeof result.excerpt, 'string');
  assert.ok(result.excerpt.length > 0);
  assert.ok(['keyword', 'semantic', 'hybrid'].includes(result.method));
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

  // Capture original signal listeners so we can restore them after daemon stops
  const originalListeners = {
    SIGTERM: process.listeners('SIGTERM').slice(),
    SIGINT: process.listeners('SIGINT').slice(),
    uncaughtException: process.listeners('uncaughtException').slice(),
    unhandledRejection: process.listeners('unhandledRejection').slice(),
  };

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

        // Remove daemon signal handlers and restore originals
        for (const [event, listeners] of Object.entries(originalListeners)) {
          // Remove all current listeners for this event
          process.removeAllListeners(event);
          // Re-add the original listeners
          for (const listener of listeners) {
            process.on(event, listener);
          }
        }
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

test('recall endpoint preserves keyword fallback ordering and required fields', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const db = loadSandboxModule(sandbox, 'db.js');
  const embeddings = loadSandboxModule(sandbox, 'embeddings.js');
  const restoreFetch = installFakeOllama(embeddings.EMBED_DIM);
  t.after(() => restoreFetch());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

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

  const recall = await httpJson(sandbox.port, 'GET', '/recall?q=alpha%20beta');
  assert.equal(recall.statusCode, 200);
  assert.ok(Array.isArray(recall.body.results));
  assert.equal(recall.body.results.length, 3);

  const orderedSources = recall.body.results.map((result) => result.source);
  assert.deepEqual(orderedSources, ['memory::alpha-beta', 'memory::alpha-only', 'memory::beta-old']);

  for (const result of recall.body.results) {
    assertRecallResultContract(result);
    assert.equal(result.method, 'keyword');
  }

  assert.ok(recall.body.results[0].relevance > recall.body.results[1].relevance);
  assert.ok(recall.body.results[1].relevance > recall.body.results[2].relevance);
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

test('core HTTP route contracts stay stable for boot/recall/store/health/digest/forget/resolve', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const health = await httpJson(sandbox.port, 'GET', '/health');
  assert.equal(health.statusCode, 200);
  assert.equal(health.body.status, 'ok');
  assert.equal(typeof health.body.stats, 'object');

  const boot = await httpJson(sandbox.port, 'GET', '/boot?agent=route-parity');
  assert.equal(boot.statusCode, 200);
  assert.equal(typeof boot.body.bootPrompt, 'string');
  assert.ok(boot.body.bootPrompt.length > 0);
  assert.equal(typeof boot.body.tokenEstimate, 'number');
  assert.ok(boot.body.profile);
  assert.ok(boot.body.capsules === null || Array.isArray(boot.body.capsules));

  const digest = await httpJson(sandbox.port, 'GET', '/digest');
  assert.equal(digest.statusCode, 200);
  assert.equal(typeof digest.body.date, 'string');
  assert.equal(typeof digest.body.totals, 'object');
  assert.equal(typeof digest.body.totals.memories, 'number');
  assert.equal(typeof digest.body.oneliner, 'string');

  const unauthStore = await httpJson(sandbox.port, 'POST', '/store', {
    body: { decision: 'unauthorized should fail' },
  });
  assert.equal(unauthStore.statusCode, 401);
  assert.equal(unauthStore.body.error, 'Unauthorized');

  const firstDecision = await httpJson(sandbox.port, 'POST', '/store', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      decision: 'route parity sentinel alpha',
      context: 'route-parity',
      source_agent: 'parity-tester',
    },
  });
  assert.equal(firstDecision.statusCode, 200);
  assert.equal(firstDecision.body.stored, true);
  assert.equal(typeof firstDecision.body.entry.id, 'number');

  const secondDecision = await httpJson(sandbox.port, 'POST', '/store', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      decision: 'route parity sentinel beta',
      context: 'route-parity',
      source_agent: 'parity-tester',
    },
  });
  assert.equal(secondDecision.statusCode, 200);
  assert.equal(secondDecision.body.stored, true);
  assert.equal(typeof secondDecision.body.entry.id, 'number');

  const missingQueryRecall = await httpJson(sandbox.port, 'GET', '/recall');
  assert.equal(missingQueryRecall.statusCode, 400);
  assert.match(missingQueryRecall.body.error, /Missing query parameter: q/);

  const recall = await httpJson(sandbox.port, 'GET', '/recall?q=parity%20sentinel');
  assert.equal(recall.statusCode, 200);
  assert.ok(Array.isArray(recall.body.results));
  assert.ok(recall.body.results.length >= 1);
  for (const result of recall.body.results) {
    assertRecallResultContract(result);
  }

  const missingForgetField = await httpJson(sandbox.port, 'POST', '/forget', {
    headers: { Authorization: `Bearer ${token}` },
    body: {},
  });
  assert.equal(missingForgetField.statusCode, 400);
  assert.equal(missingForgetField.body.error, 'Missing field: keyword');

  const forget = await httpJson(sandbox.port, 'POST', '/forget', {
    headers: { Authorization: `Bearer ${token}` },
    body: { keyword: 'parity sentinel' },
  });
  assert.equal(forget.statusCode, 200);
  assert.ok(typeof forget.body.affected === 'number');
  assert.ok(forget.body.affected >= 1);

  const missingResolveFields = await httpJson(sandbox.port, 'POST', '/resolve', {
    headers: { Authorization: `Bearer ${token}` },
    body: { keepId: firstDecision.body.entry.id },
  });
  assert.equal(missingResolveFields.statusCode, 400);
  assert.equal(missingResolveFields.body.error, 'Missing fields: keepId, action');

  const resolve = await httpJson(sandbox.port, 'POST', '/resolve', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      keepId: firstDecision.body.entry.id,
      action: 'keep',
      supersededId: secondDecision.body.entry.id,
    },
  });
  assert.equal(resolve.statusCode, 200);
  assert.equal(resolve.body.resolved, true);
});

test('MCP HTTP transport contracts stay stable', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const mcpSse = await httpJson(sandbox.port, 'GET', '/mcp');
  assert.equal(mcpSse.statusCode, 405);
  assert.equal(mcpSse.body.error, 'SSE streaming not implemented. Use POST for requests.');

  const initialize = await httpJson(sandbox.port, 'POST', '/mcp', {
    body: {
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {},
    },
  });
  assert.equal(initialize.statusCode, 200);
  assert.equal(initialize.body.jsonrpc, '2.0');
  assert.equal(initialize.body.id, 1);
  assert.equal(initialize.body.result.serverInfo.name, 'cortex');
  assert.equal(initialize.body.result.protocolVersion, '2024-11-05');
  assert.equal(initialize.headers['mcp-protocol-version'], '2024-11-05');

  const sessionId = initialize.headers['mcp-session-id'];
  assert.ok(sessionId);

  const missingSession = await httpJson(sandbox.port, 'POST', '/mcp', {
    body: {
      jsonrpc: '2.0',
      id: 2,
      method: 'tools/list',
      params: {},
    },
  });
  assert.equal(missingSession.statusCode, 400);
  assert.equal(missingSession.body.error.code, -32600);
  assert.match(missingSession.body.error.message, /Missing Mcp-Session-Id header/);

  const toolsList = await httpJson(sandbox.port, 'POST', '/mcp', {
    headers: { 'Mcp-Session-Id': sessionId },
    body: {
      jsonrpc: '2.0',
      id: 3,
      method: 'tools/list',
      params: {},
    },
  });
  assert.equal(toolsList.statusCode, 200);
  assert.equal(toolsList.body.jsonrpc, '2.0');
  assert.equal(toolsList.body.id, 3);
  assert.ok(Array.isArray(toolsList.body.result.tools));
  assert.ok(toolsList.body.result.tools.some((tool) => tool.name === 'cortex_boot'));
  assert.ok(toolsList.body.result.tools.some((tool) => tool.name === 'cortex_recall'));

  const healthCall = await httpJson(sandbox.port, 'POST', '/mcp', {
    headers: { 'Mcp-Session-Id': sessionId },
    body: {
      jsonrpc: '2.0',
      id: 4,
      method: 'tools/call',
      params: {
        name: 'cortex_health',
        arguments: {},
      },
    },
  });
  assert.equal(healthCall.statusCode, 200);
  assert.equal(healthCall.body.jsonrpc, '2.0');
  assert.equal(healthCall.body.id, 4);
  assert.ok(Array.isArray(healthCall.body.result.content));
  assert.equal(healthCall.body.result.content[0].type, 'text');

  const wrapped = JSON.parse(healthCall.body.result.content[0].text);
  assert.equal(wrapped._liveness, true);
  assert.equal(typeof wrapped.stats, 'object');
  assert.ok(['healthy', 'degraded'].includes(wrapped.overall));

  const notification = await httpJson(sandbox.port, 'POST', '/mcp', {
    headers: { 'Mcp-Session-Id': sessionId },
    body: {
      jsonrpc: '2.0',
      method: 'notifications/initialized',
    },
  });
  assert.equal(notification.statusCode, 202);
});

test('conductor and event stream route contracts stay stable', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const stream = await readFirstSseEvent(sandbox.port, '/events/stream');
  assert.equal(stream.statusCode, 200);
  assert.match(stream.headers['content-type'], /^text\/event-stream/);
  assert.equal(stream.event, 'connected');
  assert.equal(typeof stream.data.timestamp, 'string');
  assert.equal(typeof stream.data.clients, 'number');

  const lock = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: { path: '/cortex/src/daemon.js', agent: 'route-parity', ttl: 300 },
  });
  assert.equal(lock.statusCode, 200);
  assert.equal(lock.body.locked, true);
  assert.ok(lock.body.lockId);
  assert.ok(lock.body.expiresAt);

  const locks = await httpJson(sandbox.port, 'GET', '/locks', {
    headers: { Authorization: `Bearer ${token}` },
  });
  assert.equal(locks.statusCode, 200);
  assert.ok(Array.isArray(locks.body.locks));
  assert.equal(locks.body.locks.length, 1);
  assert.equal(locks.body.locks[0].agent, 'route-parity');
  assert.equal(locks.body.locks[0].path, '/cortex/src/daemon.js');
  assert.equal(typeof locks.body.locks[0].id, 'string');
  assert.equal(typeof locks.body.locks[0].expiresAt, 'string');

  const sessionStart = await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'route-parity',
      project: 'cortex',
      files: ['/src/daemon.js'],
      description: 'route parity session',
    },
  });
  assert.equal(sessionStart.statusCode, 200);
  assert.ok(sessionStart.body.sessionId);
  assert.equal(sessionStart.body.heartbeatInterval, 60);

  const sessions = await httpJson(sandbox.port, 'GET', '/sessions', {
    headers: { Authorization: `Bearer ${token}` },
  });
  assert.equal(sessions.statusCode, 200);
  assert.ok(Array.isArray(sessions.body.sessions));
  assert.equal(sessions.body.sessions.length, 1);
  assert.equal(sessions.body.sessions[0].agent, 'route-parity');
  assert.equal(typeof sessions.body.sessions[0].sessionId, 'string');
  assert.equal(sessions.body.sessions[0].project, 'cortex');
  assert.deepEqual(sessions.body.sessions[0].files, ['/src/daemon.js']);

  const taskCreate = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      title: 'route parity task',
      priority: 'high',
      requiredCapability: 'node',
    },
  });
  assert.equal(taskCreate.statusCode, 201);
  assert.ok(taskCreate.body.taskId);
  assert.equal(taskCreate.body.status, 'pending');

  const taskClaim = await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: taskCreate.body.taskId, agent: 'route-parity' },
  });
  assert.equal(taskClaim.statusCode, 200);
  assert.equal(taskClaim.body.claimed, true);

  const taskComplete = await httpJson(sandbox.port, 'POST', '/tasks/complete', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: taskCreate.body.taskId, agent: 'route-parity', summary: 'completed in parity test' },
  });
  assert.equal(taskComplete.statusCode, 200);
  assert.equal(taskComplete.body.completed, true);

  const tasks = await httpJson(sandbox.port, 'GET', '/tasks?status=all', {
    headers: { Authorization: `Bearer ${token}` },
  });
  assert.equal(tasks.statusCode, 200);
  assert.ok(Array.isArray(tasks.body.tasks));
  const completedTask = tasks.body.tasks.find((task) => task.taskId === taskCreate.body.taskId);
  assert.ok(completedTask);
  assert.equal(completedTask.title, 'route parity task');
  assert.equal(completedTask.status, 'completed');
  assert.equal(completedTask.claimedBy, 'route-parity');
  assert.equal(completedTask.requiredCapability, 'node');

  const nextTask = await httpJson(sandbox.port, 'GET', '/tasks/next?agent=route-parity&capability=node', {
    headers: { Authorization: `Bearer ${token}` },
  });
  assert.equal(nextTask.statusCode, 200);
  assert.ok(Object.hasOwn(nextTask.body, 'task'));

  const activityPost = await httpJson(sandbox.port, 'POST', '/activity', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'route-parity',
      description: 'recording activity contract',
      files: ['/src/daemon.js'],
    },
  });
  assert.equal(activityPost.statusCode, 200);
  assert.equal(activityPost.body.recorded, true);
  assert.ok(activityPost.body.activityId);

  const activityGet = await httpJson(sandbox.port, 'GET', '/activity?since=5m', {
    headers: { Authorization: `Bearer ${token}` },
  });
  assert.equal(activityGet.statusCode, 200);
  assert.ok(Array.isArray(activityGet.body.activities));
  const postedActivity = activityGet.body.activities.find((item) => item.description === 'recording activity contract');
  assert.ok(postedActivity);
  assert.equal(postedActivity.agent, 'route-parity');
  assert.equal(typeof postedActivity.id, 'string');
  assert.equal(typeof postedActivity.timestamp, 'string');

  const messagePost = await httpJson(sandbox.port, 'POST', '/message', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      from: 'route-parity',
      to: 'other-agent',
      message: 'route parity message',
    },
  });
  assert.equal(messagePost.statusCode, 200);
  assert.equal(messagePost.body.sent, true);
  assert.ok(messagePost.body.messageId);

  const messageGet = await httpJson(sandbox.port, 'GET', '/messages?agent=other-agent', {
    headers: { Authorization: `Bearer ${token}` },
  });
  assert.equal(messageGet.statusCode, 200);
  assert.ok(Array.isArray(messageGet.body.messages));
  assert.equal(messageGet.body.messages.length, 1);
  assert.equal(messageGet.body.messages[0].message, 'route parity message');
  assert.equal(messageGet.body.messages[0].from, 'route-parity');
  assert.equal(messageGet.body.messages[0].to, 'other-agent');
  assert.equal(typeof messageGet.body.messages[0].id, 'string');
  assert.equal(typeof messageGet.body.messages[0].timestamp, 'string');

  const feedPost = await httpJson(sandbox.port, 'POST', '/feed', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'route-parity',
      kind: 'completion',
      summary: 'route parity feed summary',
      content: 'detailed feed content',
      files: ['/src/daemon.js'],
    },
  });
  assert.equal(feedPost.statusCode, 201);
  assert.ok(feedPost.body.feedId);
  assert.equal(feedPost.body.recorded, true);

  const feedGet = await httpJson(sandbox.port, 'GET', '/feed?since=5m', {
    headers: { Authorization: `Bearer ${token}` },
  });
  assert.equal(feedGet.statusCode, 200);
  assert.ok(Array.isArray(feedGet.body.entries));
  const postedEntry = feedGet.body.entries.find((entry) => entry.summary === 'route parity feed summary');
  assert.ok(postedEntry);
  assert.equal(postedEntry.content, undefined);
  assert.equal(postedEntry.agent, 'route-parity');
  assert.equal(postedEntry.kind, 'completion');
  assert.equal(typeof postedEntry.id, 'string');
  assert.equal(typeof postedEntry.timestamp, 'string');

  const feedAck = await httpJson(sandbox.port, 'POST', '/feed/ack', {
    headers: { Authorization: `Bearer ${token}` },
    body: { agent: 'route-parity', lastSeenId: feedPost.body.feedId },
  });
  assert.equal(feedAck.statusCode, 200);
  assert.equal(feedAck.body.acked, true);
});

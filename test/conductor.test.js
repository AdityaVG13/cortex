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
        await sleep(250); // Increased delay for clean shutdown
      } catch {
        // Daemon may already be closed.
        await sleep(100);
      } finally {
        process.exit = originalExit;
        process.stderr.write = originalStderrWrite;
      }
    },
  };
}

// Phase 0: File Locking Tests

test('POST /lock acquires lock successfully', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const lockRes = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'factory-droid',
      ttl: 300
    }
  });

  assert.equal(lockRes.statusCode, 200);
  assert.equal(lockRes.body.locked, true);
  assert.ok(lockRes.body.lockId);
  assert.ok(lockRes.body.expiresAt);
});

test('POST /lock returns 409 when already locked', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // First agent locks the file
  const lock1 = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'claude-code',
      ttl: 300
    }
  });

  assert.equal(lock1.statusCode, 200);

  // Second agent tries to lock the same file
  const lock2 = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'factory-droid',
      ttl: 300
    }
  });

  assert.equal(lock2.statusCode, 409);
  assert.equal(lock2.body.error, 'file_already_locked');
  assert.equal(lock2.body.holder, 'claude-code');
  assert.ok(lock2.body.expiresAt);
});

test('POST /lock renews lock for same agent', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // First lock
  const lock1 = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'factory-droid',
      ttl: 300
    }
  });

  assert.equal(lock1.statusCode, 200);
  const firstExpiresAt = new Date(lock1.body.expiresAt);
  await sleep(100);

  // Renew lock (same agent)
  const lock2 = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'factory-droid',
      ttl: 300
    }
  });

  assert.equal(lock2.statusCode, 200);
  const secondExpiresAt = new Date(lock2.body.expiresAt);
  assert.ok(secondExpiresAt.getTime() > firstExpiresAt.getTime());
});

test('POST /unlock releases lock', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Acquire lock
  const lock = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'factory-droid',
      ttl: 300
    }
  });

  assert.equal(lock.statusCode, 200);

  // Release lock
  const unlock = await httpJson(sandbox.port, 'POST', '/unlock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'factory-droid'
    }
  });

  assert.equal(unlock.statusCode, 200);
  assert.equal(unlock.body.unlocked, true);

  // Should be able to lock again
  const lockAgain = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'other-agent',
      ttl: 300
    }
  });

  assert.equal(lockAgain.statusCode, 200);
});

test('POST /unlock only allowed by lock holder', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Agent A locks file
  const lock = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'agent-a',
      ttl: 300
    }
  });

  assert.equal(lock.statusCode, 200);

  // Agent B tries to unlock
  const unlock = await httpJson(sandbox.port, 'POST', '/unlock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'agent-b'
    }
  });

  assert.equal(unlock.statusCode, 403);
  assert.equal(unlock.body.error, 'not_lock_holder');
});

test('GET /locks lists all active locks', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Lock multiple files
  await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'agent-a',
      ttl: 300
    }
  });

  await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/brain.js',
      agent: 'agent-b',
      ttl: 300
    }
  });

  // List all locks
  const locksRes = await httpJson(sandbox.port, 'GET', '/locks', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(locksRes.statusCode, 200);
  assert.equal(locksRes.body.locks.length, 2);

  const paths = locksRes.body.locks.map(l => l.path).sort();
  assert.equal(paths[0], '/cortex/src/brain.js');
  assert.equal(paths[1], '/cortex/src/daemon.js');
});

test('Locks auto-expire after TTL', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Lock with short TTL (1 second)
  const lock = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'factory-droid',
      ttl: 1
    }
  });

  assert.equal(lock.statusCode, 200);

  // Wait for expiration
  await sleep(1500);

  // Should be able to lock again
  const lockAgain = await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'other-agent',
      ttl: 300
    }
  });

  assert.equal(lockAgain.statusCode, 200);
});

// Phase 0: Activity Channel Tests

test('POST /activity records agent activity', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const activityRes = await httpJson(sandbox.port, 'POST', '/activity', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'factory-droid',
      description: 'Writing Phase 0 spec',
      files: ['/cortex/docs/conductor/specs/phase-0.md']
    }
  });

  assert.equal(activityRes.statusCode, 200);
  assert.equal(activityRes.body.recorded, true);
  assert.ok(activityRes.body.activityId);
});

test('GET /activity returns only recent activities', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Record activities
  await httpJson(sandbox.port, 'POST', '/activity', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'agent-a',
      description: 'Activity 1',
      files: []
    }
  });

  await sleep(100);

  await httpJson(sandbox.port, 'POST', '/activity', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'agent-b',
      description: 'Activity 2',
      files: []
    }
  });

  // Get recent activities
  const recentRes = await httpJson(sandbox.port, 'GET', '/activity?since=5s', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(recentRes.statusCode, 200);
  assert.equal(recentRes.body.activities.length, 2);
});

test('POST /message sends inter-agent message', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const messageRes = await httpJson(sandbox.port, 'POST', '/message', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      from: 'factory-droid',
      to: 'claude-code',
      message: 'Dont touch auth.js, Im fixing CORS'
    }
  });

  assert.equal(messageRes.statusCode, 200);
  assert.equal(messageRes.body.sent, true);
  assert.ok(messageRes.body.messageId);
});

test('GET /messages returns messages for specific agent', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Send messages to different agents
  await httpJson(sandbox.port, 'POST', '/message', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      from: 'agent-a',
      to: 'agent-b',
      message: 'Message for B'
    }
  });

  await httpJson(sandbox.port, 'POST', '/message', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      from: 'agent-b',
      to: 'agent-a',
      message: 'Message for A'
    }
  });

  // Get messages for agent-a
  const messagesRes = await httpJson(sandbox.port, 'GET', '/messages?agent=agent-a', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(messagesRes.statusCode, 200);
  assert.equal(messagesRes.body.messages.length, 1);
  assert.equal(messagesRes.body.messages[0].from, 'agent-b');
  assert.equal(messagesRes.body.messages[0].to, 'agent-a');
});

// Phase 0: Boot-Injected Awareness Tests

test('Capsule compiler injects pending locks into delta capsule', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Create lock
  await httpJson(sandbox.port, 'POST', '/lock', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      path: '/cortex/src/daemon.js',
      agent: 'claude-code',
      ttl: 300
    }
  });

  // Get boot prompt
  const bootRes = await httpJson(sandbox.port, 'GET', '/boot?agent=factory-droid', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(bootRes.statusCode, 200);
  assert.match(bootRes.body.bootPrompt, /Active Locks/);
  assert.match(bootRes.body.bootPrompt, /claude-code/);
  assert.match(bootRes.body.bootPrompt, /daemon.js/);
});

test('Capsule compiler injects pending messages into delta capsule', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Send message
  await httpJson(sandbox.port, 'POST', '/message', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      from: 'claude-code',
      to: 'factory-droid',
      message: 'Review my auth changes'
    }
  });

  // Get boot prompt
  const bootRes = await httpJson(sandbox.port, 'GET', '/boot?agent=factory-droid', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(bootRes.statusCode, 200);
  assert.match(bootRes.body.bootPrompt, /Pending Messages/);
  assert.match(bootRes.body.bootPrompt, /claude-code/);
  assert.match(bootRes.body.bootPrompt, /Review my auth changes/);
});

// Session Bus Tests

test('POST /session/start registers agent session', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const res = await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      project: 'cortex',
      files: ['/src/daemon.js'],
      description: 'Implementing Session Bus'
    }
  });

  assert.equal(res.statusCode, 200);
  assert.ok(res.body.sessionId);
  assert.equal(res.body.heartbeatInterval, 60);
});

test('POST /session/start replaces existing session for same agent', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // First session
  const res1 = await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      project: 'cortex',
      files: ['/src/daemon.js'],
      description: 'First task'
    }
  });

  assert.equal(res1.statusCode, 200);
  const firstId = res1.body.sessionId;

  // Replace with new session
  const res2 = await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      project: 'cortex',
      files: ['/src/compiler.js'],
      description: 'Second task'
    }
  });

  assert.equal(res2.statusCode, 200);
  assert.notEqual(res2.body.sessionId, firstId);

  // Only one session should exist
  const listRes = await httpJson(sandbox.port, 'GET', '/sessions', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(listRes.body.sessions.length, 1);
  assert.equal(listRes.body.sessions[0].description, 'Second task');
});

test('POST /session/heartbeat renews session', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Start session
  await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      project: 'cortex',
      files: ['/src/daemon.js'],
      description: 'Working'
    }
  });

  await sleep(100);

  // Heartbeat
  const hbRes = await httpJson(sandbox.port, 'POST', '/session/heartbeat', {
    headers: { Authorization: `Bearer ${token}` },
    body: { agent: 'claude-code' }
  });

  assert.equal(hbRes.statusCode, 200);
  assert.equal(hbRes.body.renewed, true);
  assert.ok(hbRes.body.expiresAt);
});

test('POST /session/heartbeat updates files and description', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Start session
  await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      project: 'cortex',
      files: ['/src/daemon.js'],
      description: 'Starting work'
    }
  });

  // Heartbeat with updated info
  await httpJson(sandbox.port, 'POST', '/session/heartbeat', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      files: ['/src/compiler.js', '/src/brain.js'],
      description: 'Moved to compiler'
    }
  });

  // Verify update
  const listRes = await httpJson(sandbox.port, 'GET', '/sessions', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(listRes.body.sessions.length, 1);
  assert.deepEqual(listRes.body.sessions[0].files, ['/src/compiler.js', '/src/brain.js']);
  assert.equal(listRes.body.sessions[0].description, 'Moved to compiler');
});

test('POST /session/heartbeat returns 404 for no active session', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const hbRes = await httpJson(sandbox.port, 'POST', '/session/heartbeat', {
    headers: { Authorization: `Bearer ${token}` },
    body: { agent: 'ghost-agent' }
  });

  assert.equal(hbRes.statusCode, 404);
  assert.equal(hbRes.body.error, 'no_active_session');
});

test('POST /session/end removes session', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Start then end
  await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      project: 'cortex',
      files: [],
      description: 'Temporary'
    }
  });

  const endRes = await httpJson(sandbox.port, 'POST', '/session/end', {
    headers: { Authorization: `Bearer ${token}` },
    body: { agent: 'claude-code' }
  });

  assert.equal(endRes.statusCode, 200);
  assert.equal(endRes.body.ended, true);

  // Verify gone
  const listRes = await httpJson(sandbox.port, 'GET', '/sessions', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(listRes.body.sessions.length, 0);
});

test('GET /sessions lists active sessions', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Register two agents
  await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      project: 'cortex',
      files: ['/src/daemon.js'],
      description: 'Session Bus'
    }
  });

  await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'factory-droid',
      project: 'cortex',
      files: ['/workers/dash.py'],
      description: 'Dashboard MVP'
    }
  });

  const listRes = await httpJson(sandbox.port, 'GET', '/sessions', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(listRes.statusCode, 200);
  assert.equal(listRes.body.sessions.length, 2);

  const agents = listRes.body.sessions.map(s => s.agent).sort();
  assert.deepEqual(agents, ['claude-code', 'factory-droid']);
});

test('Sessions auto-expire after TTL', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Start session with very short TTL (1 second)
  await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'claude-code',
      project: 'cortex',
      files: [],
      description: 'Short-lived',
      ttl: 1
    }
  });

  // Should be active immediately
  const before = await httpJson(sandbox.port, 'GET', '/sessions', {
    headers: { Authorization: `Bearer ${token}` }
  });
  assert.equal(before.body.sessions.length, 1);

  // Wait for expiry
  await sleep(1500);

  // Should be gone
  const after = await httpJson(sandbox.port, 'GET', '/sessions', {
    headers: { Authorization: `Bearer ${token}` }
  });
  assert.equal(after.body.sessions.length, 0);
});

test('Boot prompt injects active agent sessions', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Register another agent
  await httpJson(sandbox.port, 'POST', '/session/start', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      agent: 'factory-droid',
      project: 'cortex',
      files: ['/workers/dash.py'],
      description: 'Building dashboard'
    }
  });

  // Get boot prompt as claude-code
  const bootRes = await httpJson(sandbox.port, 'GET', '/boot?agent=claude-code', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(bootRes.statusCode, 200);
  assert.match(bootRes.body.bootPrompt, /Active Agents/);
  assert.match(bootRes.body.bootPrompt, /factory-droid/);
  assert.match(bootRes.body.bootPrompt, /Building dashboard/);
});

// Task Board Tests

test('POST /tasks creates a new task', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const res = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      title: 'Add rate limiting',
      description: 'Limit /store to 60 calls/min',
      project: 'cortex',
      files: ['/src/daemon.js'],
      priority: 'high',
      requiredCapability: 'node'
    }
  });

  assert.equal(res.statusCode, 201);
  assert.ok(res.body.taskId);
  assert.equal(res.body.status, 'pending');
});

test('GET /tasks lists pending tasks', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Create two tasks
  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Task A', priority: 'high' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Task B', priority: 'low' }
  });

  const res = await httpJson(sandbox.port, 'GET', '/tasks', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(res.statusCode, 200);
  assert.equal(res.body.tasks.length, 2);
});

test('GET /tasks filters by status', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Create and claim a task
  const createRes = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Claimable task', priority: 'medium' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'claude-code' }
  });

  // Create another pending task
  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Pending task', priority: 'low' }
  });

  // Filter pending only
  const pending = await httpJson(sandbox.port, 'GET', '/tasks?status=pending', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(pending.body.tasks.length, 1);
  assert.equal(pending.body.tasks[0].title, 'Pending task');

  // Filter claimed only
  const claimed = await httpJson(sandbox.port, 'GET', '/tasks?status=claimed', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(claimed.body.tasks.length, 1);
  assert.equal(claimed.body.tasks[0].title, 'Claimable task');
});

test('POST /tasks/claim claims a task', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const createRes = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Claim me', priority: 'high' }
  });

  const claimRes = await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'claude-code' }
  });

  assert.equal(claimRes.statusCode, 200);
  assert.equal(claimRes.body.claimed, true);
});

test('POST /tasks/claim returns 409 when already claimed', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const createRes = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Contested task', priority: 'high' }
  });

  // First agent claims
  await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'claude-code' }
  });

  // Second agent tries
  const res = await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'factory-droid' }
  });

  assert.equal(res.statusCode, 409);
  assert.equal(res.body.error, 'task_already_claimed');
  assert.equal(res.body.claimedBy, 'claude-code');
});

test('POST /tasks/complete marks task done', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const createRes = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Complete me', priority: 'medium' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'claude-code' }
  });

  const completeRes = await httpJson(sandbox.port, 'POST', '/tasks/complete', {
    headers: { Authorization: `Bearer ${token}` },
    body: {
      taskId: createRes.body.taskId,
      agent: 'claude-code',
      summary: 'Done with tests'
    }
  });

  assert.equal(completeRes.statusCode, 200);
  assert.equal(completeRes.body.completed, true);
});

test('POST /tasks/complete only allowed by holder', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const createRes = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Guarded task', priority: 'high' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'claude-code' }
  });

  // Different agent tries to complete
  const res = await httpJson(sandbox.port, 'POST', '/tasks/complete', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'factory-droid', summary: 'Impostor' }
  });

  assert.equal(res.statusCode, 403);
  assert.equal(res.body.error, 'not_task_holder');
});

test('POST /tasks/abandon returns task to pending', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const createRes = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Abandonable', priority: 'low' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'claude-code' }
  });

  const abandonRes = await httpJson(sandbox.port, 'POST', '/tasks/abandon', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'claude-code' }
  });

  assert.equal(abandonRes.statusCode, 200);
  assert.equal(abandonRes.body.abandoned, true);
  assert.equal(abandonRes.body.status, 'pending');

  // Should be claimable again
  const reclaimRes = await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: createRes.body.taskId, agent: 'factory-droid' }
  });

  assert.equal(reclaimRes.statusCode, 200);
});

test('GET /tasks/next returns highest priority unclaimed task', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Create tasks with different priorities
  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Low task', priority: 'low' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Critical task', priority: 'critical' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Medium task', priority: 'medium' }
  });

  const res = await httpJson(sandbox.port, 'GET', '/tasks/next?agent=claude-code', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(res.statusCode, 200);
  assert.equal(res.body.task.title, 'Critical task');
});

test('GET /tasks/next filters by capability', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Node task', priority: 'high', requiredCapability: 'node' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Python task', priority: 'critical', requiredCapability: 'python' }
  });

  // Request node capability — should skip the critical python task
  const res = await httpJson(sandbox.port, 'GET', '/tasks/next?agent=claude-code&capability=node', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(res.statusCode, 200);
  assert.equal(res.body.task.title, 'Node task');
});

test('GET /tasks/next returns null when no tasks available', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  const res = await httpJson(sandbox.port, 'GET', '/tasks/next?agent=claude-code', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(res.statusCode, 200);
  assert.equal(res.body.task, null);
});

test('Boot prompt injects pending and claimed tasks', { concurrency: false }, async (t) => {
  const sandbox = await createSandbox();
  t.after(() => sandbox.cleanup());

  const daemon = await startDaemonInProcess(sandbox);
  const tokenPath = path.join(sandbox.homeDir, '.cortex', 'cortex.token');

  t.after(async () => {
    const token = fs.existsSync(tokenPath) ? fs.readFileSync(tokenPath, 'utf-8').trim() : null;
    await daemon.stop(token);
  });

  const token = fs.readFileSync(tokenPath, 'utf-8').trim();

  // Create a pending task
  await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'Unclaimed work', priority: 'high' }
  });

  // Create and claim a task for claude-code
  const myTask = await httpJson(sandbox.port, 'POST', '/tasks', {
    headers: { Authorization: `Bearer ${token}` },
    body: { title: 'My active task', priority: 'medium' }
  });

  await httpJson(sandbox.port, 'POST', '/tasks/claim', {
    headers: { Authorization: `Bearer ${token}` },
    body: { taskId: myTask.body.taskId, agent: 'claude-code' }
  });

  // Boot as claude-code
  const bootRes = await httpJson(sandbox.port, 'GET', '/boot?agent=claude-code', {
    headers: { Authorization: `Bearer ${token}` }
  });

  assert.equal(bootRes.statusCode, 200);
  assert.match(bootRes.body.bootPrompt, /Pending Tasks/);
  assert.match(bootRes.body.bootPrompt, /Unclaimed work/);
  assert.match(bootRes.body.bootPrompt, /Your Active Tasks/);
  assert.match(bootRes.body.bootPrompt, /My active task/);
});

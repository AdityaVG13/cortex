'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { run } = require('node:test');
const { spec } = require('node:test/reporters');

const testDir = __dirname;

const files = fs.readdirSync(testDir)
  .sort()
  .filter((file) => file.endsWith('.test.js'))
  .map((file) => path.join(testDir, file));

// Track exit state
let exited = false;
let failed = false;

const stream = run({
  files,
  concurrency: 1,
  isolation: 'none',
  forceExit: true,
});

stream.on('test:fail', () => {
  failed = true;
});

// Safety: force exit after 2 minutes if still running (tests take ~1 min)
setTimeout(() => {
  if (!exited) {
    exited = true;
    process.exit(failed ? 1 : 0);
  }
}, 120000).unref();

stream.pipe(spec()).pipe(process.stdout);

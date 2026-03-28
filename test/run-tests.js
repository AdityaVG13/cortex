'use strict';

const fs = require('node:fs');
const path = require('node:path');

const testDir = __dirname;

for (const file of fs.readdirSync(testDir).sort()) {
  if (!file.endsWith('.test.js')) continue;
  require(path.join(testDir, file));
}

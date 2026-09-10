#!/usr/bin/env node
// Docs gate: (1) retired vocabulary must not appear in the shipped guides or
// the proposal README; (2) every relative markdown link in those files must
// resolve. Legacy surfaces (boot capsule sigils, "semantic recall" tool
// alias) are allowed only where they are labeled compatibility.
const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..');
const FILES = [
  'docs/guides/user-guide.md',
  'docs/guides/developer-guide.md',
  'docs/guides/operations-guide.md',
  'docs/architecture/next/README.md',
  'docs/internal/reader-experiment-protocol.md',
];
const RETIRED = [/semantic recall/i, /saved tokens/i, /FACT[!?~]/, /\bsingle status\b/i, /Revision 3/, /\bV3\b/];

let failures = 0;
for (const rel of FILES) {
  const file = path.join(ROOT, rel);
  if (!fs.existsSync(file)) {
    console.error(`missing: ${rel}`);
    failures += 1;
    continue;
  }
  const text = fs.readFileSync(file, 'utf8');
  for (const term of RETIRED) {
    const m = text.match(term);
    if (m) {
      console.error(`${rel}: retired term ${term} ("${m[0]}")`);
      failures += 1;
    }
  }
  const links = [...text.matchAll(/\]\(([^)]+)\)/g)].map((m) => m[1]).filter((l) => !/^https?:|^#|^mailto:/.test(l));
  for (const link of links) {
    const target = path.resolve(path.dirname(file), link.split('#')[0]);
    if (!fs.existsSync(target)) {
      console.error(`${rel}: broken link ${link}`);
      failures += 1;
    }
  }
}
if (failures) {
  console.error(`${failures} docs gate failure(s)`);
  process.exit(1);
}
console.log(`docs gate ok (${FILES.length} files)`);

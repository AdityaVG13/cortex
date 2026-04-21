import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const SRC_DIR = path.dirname(fileURLToPath(import.meta.url));
const SOURCE_EXTENSIONS = new Set([".js", ".jsx"]);

function listSourceFiles() {
  return fs
    .readdirSync(SRC_DIR, { withFileTypes: true })
    .filter((entry) => entry.isFile() && SOURCE_EXTENSIONS.has(path.extname(entry.name)))
    .map((entry) => path.resolve(SRC_DIR, entry.name));
}

function resolveLocalImport(fromFile, specifier, allFiles) {
  const base = path.resolve(path.dirname(fromFile), specifier);
  const candidates = [
    base,
    `${base}.js`,
    `${base}.jsx`,
    path.join(base, "index.js"),
    path.join(base, "index.jsx"),
  ];
  return candidates.find((candidate) => allFiles.has(candidate)) ?? null;
}

function buildGraph(files) {
  const allFiles = new Set(files);
  const importPattern = /import\s+(?:[^"';]+\s+from\s+)?["'](\.[^"']+)["']/g;
  const graph = new Map();

  for (const file of files) {
    const contents = fs.readFileSync(file, "utf8");
    const dependencies = [];
    for (const match of contents.matchAll(importPattern)) {
      const resolved = resolveLocalImport(file, match[1], allFiles);
      if (resolved) dependencies.push(resolved);
    }
    graph.set(file, dependencies);
  }

  return graph;
}

function findCycles(graph) {
  const visiting = new Set();
  const visited = new Set();
  const cycles = [];
  const stack = [];
  const seenCycleKeys = new Set();

  function keyForCycle(cycle) {
    const normalized = [...cycle].sort();
    return normalized.join("|");
  }

  function visit(node) {
    if (visited.has(node)) return;
    visiting.add(node);
    stack.push(node);

    for (const next of graph.get(node) ?? []) {
      if (!visiting.has(next)) {
        visit(next);
        continue;
      }

      const cycleStartIndex = stack.lastIndexOf(next);
      if (cycleStartIndex === -1) continue;
      const cycle = stack.slice(cycleStartIndex);
      const cycleKey = keyForCycle(cycle);
      if (!seenCycleKeys.has(cycleKey)) {
        seenCycleKeys.add(cycleKey);
        cycles.push(cycle.map((entry) => path.basename(entry)));
      }
    }

    stack.pop();
    visiting.delete(node);
    visited.add(node);
  }

  for (const node of graph.keys()) {
    visit(node);
  }

  return cycles;
}

describe("module graph", () => {
  it("has no local import cycles", () => {
    const files = listSourceFiles();
    const graph = buildGraph(files);
    const cycles = findCycles(graph);
    expect(cycles).toEqual([]);
  });
});

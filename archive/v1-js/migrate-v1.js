#!/usr/bin/env node
/**
 * Migrate Cortex v1 data to v2 SQLite database.
 * Reads: cortex-index.json, embeddings.json, decisions.jsonl, events.jsonl
 * From: ~/self-improvement-engine/tools/cortex/
 */

const path = require('path');
const fs = require('fs');
const db = require('../src/db');
const { vectorToBlob } = require('../src/embeddings');

const HOME = process.env.USERPROFILE || process.env.HOME;
const V1_DIR = path.join(HOME, 'self-improvement-engine', 'tools', 'cortex');

async function migrate() {
  console.log('=== Cortex v1 → v2 Migration ===\n');

  // Verify source files exist
  const files = {
    index: path.join(V1_DIR, 'cortex-index.json'),
    embeddings: path.join(V1_DIR, 'embeddings.json'),
    decisions: path.join(V1_DIR, 'decisions.jsonl'),
    events: path.join(V1_DIR, 'events.jsonl'),
  };

  for (const [name, filePath] of Object.entries(files)) {
    if (!fs.existsSync(filePath)) {
      console.warn(`WARNING: ${name} not found at ${filePath}`);
    } else {
      const size = fs.statSync(filePath).size;
      console.log(`Found ${name}: ${(size / 1024).toFixed(1)}KB`);
    }
  }

  // Initialize database
  await db.getDb();
  console.log('\nDatabase initialized.\n');

  let counts = { memories: 0, decisions: 0, embeddings: 0, events: 0 };

  // 1. Migrate memories from cortex-index.json
  if (fs.existsSync(files.index)) {
    console.log('Migrating memories from cortex-index.json...');
    const index = JSON.parse(fs.readFileSync(files.index, 'utf8'));

    if (index.sections?.memories) {
      for (const mem of index.sections.memories) {
        db.insert(
          'INSERT INTO memories (text, source, type, source_agent, confidence, created_at) VALUES (?, ?, ?, ?, ?, ?)',
          [
            mem.content || mem.description || '',
            mem.file || 'v1-index',
            mem.type || 'memory',
            'claude-opus-v1',
            0.8,
            mem.mtime ? new Date(mem.mtime).toISOString() : new Date().toISOString()
          ]
        );
        counts.memories++;
      }
    }

    // Migrate state excerpt as a memory
    if (index.sections?.state) {
      db.insert(
        'INSERT INTO memories (text, source, type, source_agent) VALUES (?, ?, ?, ?)',
        [index.sections.state, 'state.md', 'state', 'claude-opus-v1']
      );
      counts.memories++;
    }

    // Migrate lessons
    if (index.sections?.lessons) {
      for (const lesson of index.sections.lessons) {
        db.insert(
          'INSERT INTO memories (text, source, type, source_agent, confidence) VALUES (?, ?, ?, ?, ?)',
          [
            lesson.lesson || lesson.text || '',
            `lesson:${lesson.skill || 'general'}`,
            'lesson',
            'claude-opus-v1',
            lesson.confidence || 0.5
          ]
        );
        counts.memories++;
      }
    }

    console.log(`  → ${counts.memories} memories imported`);
  }

  // 2. Migrate decisions from decisions.jsonl
  if (fs.existsSync(files.decisions)) {
    console.log('Migrating decisions from decisions.jsonl...');
    const lines = fs.readFileSync(files.decisions, 'utf8').trim().split('\n');
    for (const line of lines) {
      if (!line.trim()) continue;
      try {
        const d = JSON.parse(line);
        db.insert(
          'INSERT INTO decisions (decision, context, type, source_agent, confidence, surprise, score, retrievals, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)',
          [
            d.decision,
            d.context || null,
            d.type || 'decision',
            'claude-opus-v1',
            0.8,
            d.surprise || 1.0,
            d.score || 1.0,
            d.retrievals || 0,
            d.timestamp || new Date().toISOString()
          ]
        );
        counts.decisions++;
      } catch (e) {
        console.warn(`  Skipping malformed decision line: ${e.message}`);
      }
    }
    console.log(`  → ${counts.decisions} decisions imported`);
  }

  // 3. Migrate embeddings from embeddings.json
  if (fs.existsSync(files.embeddings)) {
    console.log('Migrating embeddings from embeddings.json...');
    const embData = JSON.parse(fs.readFileSync(files.embeddings, 'utf8'));

    for (const [key, entry] of Object.entries(embData)) {
      if (!entry.vec || !Array.isArray(entry.vec)) continue;

      // Convert JSON array to binary BLOB
      const vec = new Float32Array(entry.vec);
      const blob = vectorToBlob(vec);

      // Find matching memory or decision by source text
      let targetType = 'memory';
      let targetId = null;

      // Try to match by source
      if (entry.source) {
        const mem = db.get(
          'SELECT id FROM memories WHERE source = ? OR text LIKE ? LIMIT 1',
          [entry.source, `%${entry.text?.substring(0, 50) || ''}%`]
        );
        if (mem) {
          targetId = mem.id;
          targetType = 'memory';
        }
      }

      // Try matching decisions
      if (!targetId && entry.source?.startsWith('decision/')) {
        const dec = db.get('SELECT id FROM decisions LIMIT 1');
        if (dec) {
          targetId = dec.id;
          targetType = 'decision';
        }
      }

      // Default: create an orphan mapping to id 0 (will be cleaned up)
      if (!targetId) targetId = 0;

      db.insert(
        'INSERT INTO embeddings (target_type, target_id, vector, model) VALUES (?, ?, ?, ?)',
        [targetType, targetId, blob, 'nomic-embed-text']
      );
      counts.embeddings++;
    }
    console.log(`  → ${counts.embeddings} embeddings imported (${vec_dim(embData)})`);
  }

  // 4. Migrate events from events.jsonl
  if (fs.existsSync(files.events)) {
    console.log('Migrating events from events.jsonl...');
    const lines = fs.readFileSync(files.events, 'utf8').trim().split('\n');
    for (const line of lines) {
      if (!line.trim()) continue;
      try {
        const e = JSON.parse(line);
        db.insert(
          'INSERT INTO events (type, data, source_agent, created_at) VALUES (?, ?, ?, ?)',
          [
            e.type || 'unknown',
            JSON.stringify(e.data || e),
            'claude-opus-v1',
            e.timestamp || new Date().toISOString()
          ]
        );
        counts.events++;
      } catch (e) {
        console.warn(`  Skipping malformed event line: ${e.message}`);
      }
    }
    console.log(`  → ${counts.events} events imported`);
  }

  // Persist and report
  db.persist();

  console.log('\n=== Migration Complete ===');
  console.log(`Memories:   ${counts.memories}`);
  console.log(`Decisions:  ${counts.decisions}`);
  console.log(`Embeddings: ${counts.embeddings}`);
  console.log(`Events:     ${counts.events}`);

  // Verify
  const stats = db.getStats();
  console.log('\nDatabase verification:');
  console.log(JSON.stringify(stats, null, 2));

  db.close();
}

function vec_dim(embData) {
  for (const entry of Object.values(embData)) {
    if (entry.vec && Array.isArray(entry.vec)) {
      return `${entry.vec.length}-dim vectors`;
    }
  }
  return 'unknown dimensions';
}

migrate().catch(e => {
  console.error('Migration failed:', e);
  process.exit(1);
});

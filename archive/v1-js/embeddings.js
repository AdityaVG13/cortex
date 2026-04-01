'use strict';

const { exec } = require('child_process');
const { query, get, run, insert, persist } = require('./db');

const OLLAMA_URL = 'http://localhost:11434';
const EMBED_MODEL = 'nomic-embed-text';
const MAX_INPUT_CHARS = 512;
const EMBED_DIM = 768;
const TIMEOUT_MS = 10_000;

// Ollama state tracking
let ollamaStatus = 'unknown'; // 'online' | 'offline' | 'starting' | 'unknown'
let lastStatusLog = 0;
let consecutiveFailures = 0;
const MAX_FAILURES_BEFORE_OFFLINE = 3;
const STATUS_LOG_INTERVAL = 60000; // Only log status once per minute

/**
 * Check if Ollama is running, optionally try to start it.
 * Returns true if Ollama is available.
 */
async function ensureOllamaRunning() {
  // If we know it's online, quick check
  if (ollamaStatus === 'online') {
    return true;
  }

  // If we're already trying to start it, wait
  if (ollamaStatus === 'starting') {
    return false;
  }

  // Try to connect
  try {
    const res = await fetch(`${OLLAMA_URL}/api/tags`, { method: 'GET', signal: AbortSignal.timeout(2000) });
    if (res.ok) {
      if (ollamaStatus !== 'online' && Date.now() - lastStatusLog > STATUS_LOG_INTERVAL) {
        console.error('[embeddings] Ollama connected');
        lastStatusLog = Date.now();
      }
      ollamaStatus = 'online';
      consecutiveFailures = 0;
      return true;
    }
  } catch {
    // Ollama not responding
  }

  // If offline and not recently tried, attempt to start
  if (ollamaStatus !== 'starting' && consecutiveFailures >= MAX_FAILURES_BEFORE_OFFLINE) {
    if (Date.now() - lastStatusLog > STATUS_LOG_INTERVAL) {
      console.error('[embeddings] Ollama offline - attempting to start...');
      lastStatusLog = Date.now();
    }
    ollamaStatus = 'starting';

    // Try to start Ollama (Windows)
    const ollamaPath = process.env.LOCALAPPDATA
      ? `${process.env.LOCALAPPDATA}\\Programs\\Ollama\\ollama.exe`
      : 'ollama';

    exec(`"${ollamaPath}" app`, (err) => {
      if (err && Date.now() - lastStatusLog > STATUS_LOG_INTERVAL) {
        console.error('[embeddings] Could not start Ollama:', err.message);
        lastStatusLog = Date.now();
      }
    });

    // Give it a moment to start
    await new Promise(r => setTimeout(r, 3000));
    ollamaStatus = 'unknown'; // Will be checked next call
  }

  return false;
}

/**
 * Get embedding vector from Ollama for a text string.
 * Truncates input to 512 chars. Returns Float32Array (768-dim) or null on error.
 */
async function getEmbedding(text) {
  if (!text || typeof text !== 'string') return null;

  // Only try if Ollama might be available
  if (ollamaStatus === 'offline' && consecutiveFailures > 10) {
    return null; // Give up until status reset
  }

  const truncated = text.slice(0, MAX_INPUT_CHARS);

  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

    const res = await fetch(`${OLLAMA_URL}/api/embeddings`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ model: EMBED_MODEL, prompt: truncated }),
      signal: controller.signal,
    });

    clearTimeout(timer);

    if (!res.ok) {
      consecutiveFailures++;
      return null;
    }

    const data = await res.json();
    if (!data.embedding || !Array.isArray(data.embedding)) {
      consecutiveFailures++;
      return null;
    }

    ollamaStatus = 'online';
    consecutiveFailures = 0;
    return new Float32Array(data.embedding);
  } catch (err) {
    consecutiveFailures++;
    if (consecutiveFailures === MAX_FAILURES_BEFORE_OFFLINE) {
      ollamaStatus = 'offline';
      if (Date.now() - lastStatusLog > STATUS_LOG_INTERVAL) {
        console.error('[embeddings] Ollama offline - embeddings disabled until restart');
        lastStatusLog = Date.now();
      }
    }
    return null;
  }
}

/**
 * Convert Float32Array → Buffer for SQLite BLOB storage.
 */
function vectorToBlob(vec) {
  if (!(vec instanceof Float32Array)) {
    throw new TypeError('vectorToBlob expects a Float32Array');
  }
  return Buffer.from(vec.buffer, vec.byteOffset, vec.byteLength);
}

/**
 * Convert Buffer (from SQLite BLOB) → Float32Array for computation.
 */
function blobToVector(blob) {
  if (!Buffer.isBuffer(blob) && !(blob instanceof Uint8Array)) {
    throw new TypeError('blobToVector expects a Buffer or Uint8Array');
  }
  const buf = Buffer.isBuffer(blob) ? blob : Buffer.from(blob);
  return new Float32Array(buf.buffer, buf.byteOffset, buf.byteLength / Float32Array.BYTES_PER_ELEMENT);
}

/**
 * Cosine similarity between two BLOB-encoded vectors.
 * Accepts Uint8Array or Buffer (each storing a Float32Array).
 * Returns 0-1 (clamped). Returns 0 on invalid input.
 */
function cosineSim(a, b) {
  try {
    const vecA = blobToVector(a);
    const vecB = blobToVector(b);

    if (vecA.length !== vecB.length || vecA.length === 0) return 0;

    let dot = 0;
    let normA = 0;
    let normB = 0;

    for (let i = 0; i < vecA.length; i++) {
      dot += vecA[i] * vecB[i];
      normA += vecA[i] * vecA[i];
      normB += vecB[i] * vecB[i];
    }

    const denom = Math.sqrt(normA) * Math.sqrt(normB);
    if (denom === 0) return 0;

    // Clamp to [0, 1] — embeddings can produce slightly negative cosine
    return Math.max(0, Math.min(1, dot / denom));
  } catch {
    return 0;
  }
}

/**
 * Build embeddings for all un-embedded memories and decisions.
 * Reads rows that lack an entry in the embeddings table, computes vectors
 * via Ollama in parallel batches, and stores as BLOBs.
 * Returns { total, computed }.
 */
async function buildEmbeddings() {
  // Find memories without embeddings
  const unembeddedMemories = query(`
    SELECT m.id, m.text FROM memories m
    WHERE m.status = 'active'
      AND NOT EXISTS (
        SELECT 1 FROM embeddings e
        WHERE e.target_type = 'memory' AND e.target_id = m.id
      )
  `);

  // Find decisions without embeddings
  const unembeddedDecisions = query(`
    SELECT d.id, d.decision AS text FROM decisions d
    WHERE d.status = 'active'
      AND NOT EXISTS (
        SELECT 1 FROM embeddings e
        WHERE e.target_type = 'decision' AND e.target_id = d.id
      )
  `);

  const total = unembeddedMemories.length + unembeddedDecisions.length;
  let computed = 0;

  // Batch size for parallel requests (tune based on Ollama capacity)
  const BATCH_SIZE = 8;

  // Helper to process a batch in parallel
  async function processBatch(items, targetType) {
    const results = await Promise.all(
      items.map(async (item) => {
        try {
          const vec = await getEmbedding(item.text);
          if (vec) {
            insert(
              'INSERT INTO embeddings (target_type, target_id, vector, model) VALUES (?, ?, ?, ?)',
              [targetType, item.id, vectorToBlob(vec), EMBED_MODEL]
            );
            return 1; // Computed successfully
          }
        } catch (err) {
          console.error(`[embeddings] Failed to embed ${targetType} id ${item.id}: ${err.message}`);
        }
        return 0; // Failed or skipped
      })
    );
    return results.reduce((sum, val) => sum + val, 0);
  }

  // Process memories in batches
  for (let i = 0; i < unembeddedMemories.length; i += BATCH_SIZE) {
    const batch = unembeddedMemories.slice(i, i + BATCH_SIZE);
    computed += await processBatch(batch, 'memory');
  }

  // Process decisions in batches
  for (let i = 0; i < unembeddedDecisions.length; i += BATCH_SIZE) {
    const batch = unembeddedDecisions.slice(i, i + BATCH_SIZE);
    computed += await processBatch(batch, 'decision');
  }

  if (computed > 0) persist();

  return { total, computed };
}

module.exports = {
  getEmbedding,
  cosineSim,
  vectorToBlob,
  blobToVector,
  buildEmbeddings,
  EMBED_DIM,
  OLLAMA_URL,
  EMBED_MODEL,
};

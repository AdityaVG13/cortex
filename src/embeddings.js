'use strict';

const { query, get, run, insert, persist } = require('./db');

const OLLAMA_URL = 'http://localhost:11434';
const EMBED_MODEL = 'nomic-embed-text';
const MAX_INPUT_CHARS = 512;
const EMBED_DIM = 768;
const TIMEOUT_MS = 10_000;

/**
 * Get embedding vector from Ollama for a text string.
 * Truncates input to 512 chars. Returns Float32Array (768-dim) or null on error.
 */
async function getEmbedding(text) {
  if (!text || typeof text !== 'string') return null;

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
      console.error(`[embeddings] Ollama returned ${res.status}: ${res.statusText}`);
      return null;
    }

    const data = await res.json();
    if (!data.embedding || !Array.isArray(data.embedding)) {
      console.error('[embeddings] Unexpected Ollama response shape');
      return null;
    }

    return new Float32Array(data.embedding);
  } catch (err) {
    if (err.name === 'AbortError') {
      console.error('[embeddings] Ollama request timed out');
    } else {
      console.error(`[embeddings] Ollama error: ${err.message}`);
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
 * via Ollama, and stores as BLOBs.
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

  // Process memories
  for (const row of unembeddedMemories) {
    const vec = await getEmbedding(row.text);
    if (vec) {
      insert(
        'INSERT INTO embeddings (target_type, target_id, vector, model) VALUES (?, ?, ?, ?)',
        ['memory', row.id, vectorToBlob(vec), EMBED_MODEL]
      );
      computed++;
    }
  }

  // Process decisions
  for (const row of unembeddedDecisions) {
    const vec = await getEmbedding(row.text);
    if (vec) {
      insert(
        'INSERT INTO embeddings (target_type, target_id, vector, model) VALUES (?, ?, ?, ?)',
        ['decision', row.id, vectorToBlob(vec), EMBED_MODEL]
      );
      computed++;
    }
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

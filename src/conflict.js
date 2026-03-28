const db = require('./db');
const embeddings = require('./embeddings');

// ── Jaccard fallback (when Ollama is down) ──────────────────────────

function tokenize(text) {
  return new Set(
    text
      .toLowerCase()
      .split(/\W+/)
      .filter(w => w.length > 2)
  );
}

function jaccardSimilarity(textA, textB) {
  const a = tokenize(textA);
  const b = tokenize(textB);
  if (a.size === 0 && b.size === 0) return 1;
  if (a.size === 0 || b.size === 0) return 0;

  let intersection = 0;
  for (const word of a) {
    if (b.has(word)) intersection++;
  }
  const union = a.size + b.size - intersection;
  return union === 0 ? 0 : intersection / union;
}

// ── Core detection ──────────────────────────────────────────────────

/**
 * Detect whether newText conflicts with any active decision.
 *
 * Uses cosine similarity on embeddings when Ollama is up,
 * falls back to Jaccard word similarity when it is not.
 *
 * @param {string} newText    - The text to check against existing decisions
 * @param {string} sourceAgent - The agent that produced newText
 * @returns {Promise<{isConflict: boolean, isUpdate: boolean, matchedId?: number, similarity?: number, matchedAgent?: string}>}
 */
async function detectConflict(newText, sourceAgent) {
  const newEmbedding = await embeddings.getEmbedding(newText);
  const useEmbeddings = newEmbedding !== null;

  // Load all active decisions
  const decisions = db.query(
    "SELECT id, decision, source_agent FROM decisions WHERE status = 'active'"
  );

  if (decisions.length === 0) {
    return { isConflict: false, isUpdate: false };
  }

  // If we have embeddings, load them and use cosine similarity
  if (useEmbeddings) {
    const COSINE_THRESHOLD = 0.85;
    let bestMatch = null;
    let bestSim = -1;

    for (const dec of decisions) {
      const row = db.get(
        "SELECT vector FROM embeddings WHERE target_type = 'decision' AND target_id = ?",
        [dec.id]
      );
      if (!row || !row.vector) continue;

      const existingVector = embeddings.blobToVector(row.vector);
      const sim = embeddings.cosineSim(newEmbedding, existingVector);

      if (sim > bestSim) {
        bestSim = sim;
        bestMatch = dec;
      }
    }

    if (bestMatch && bestSim > COSINE_THRESHOLD) {
      if (bestMatch.source_agent === sourceAgent) {
        return { isConflict: false, isUpdate: true, matchedId: bestMatch.id, similarity: bestSim };
      }
      return {
        isConflict: true,
        isUpdate: false,
        matchedId: bestMatch.id,
        similarity: bestSim,
        matchedAgent: bestMatch.source_agent,
      };
    }

    return { isConflict: false, isUpdate: false };
  }

  // ── Jaccard fallback ────────────────────────────────────────────
  const JACCARD_THRESHOLD = 0.6;
  let bestMatch = null;
  let bestSim = -1;

  for (const dec of decisions) {
    const sim = jaccardSimilarity(newText, dec.decision);
    if (sim > bestSim) {
      bestSim = sim;
      bestMatch = dec;
    }
  }

  if (bestMatch && bestSim > JACCARD_THRESHOLD) {
    if (bestMatch.source_agent === sourceAgent) {
      return { isConflict: false, isUpdate: true, matchedId: bestMatch.id, similarity: bestSim };
    }
    return {
      isConflict: true,
      isUpdate: false,
      matchedId: bestMatch.id,
      similarity: bestSim,
      matchedAgent: bestMatch.source_agent,
    };
  }

  return { isConflict: false, isUpdate: false };
}

// ── Dispute management ──────────────────────────────────────────────

/**
 * Mark two entries as disputed (each references the other).
 *
 * @param {number} newId      - The newer decision id
 * @param {number} existingId - The existing decision id it conflicts with
 */
function markDisputed(newId, existingId) {
  db.run(
    "UPDATE decisions SET status = 'disputed', disputes_id = ? WHERE id = ?",
    [existingId, newId]
  );
  db.run(
    "UPDATE decisions SET status = 'disputed', disputes_id = ? WHERE id = ?",
    [newId, existingId]
  );
  db.persist();
}

// ── Resolution ──────────────────────────────────────────────────────

/**
 * Resolve a dispute between two decisions.
 *
 * @param {number} keepId       - The decision to keep / prioritise
 * @param {'keep'|'merge'} action - 'keep' supersedes the other; 'merge' keeps both active
 * @param {number} supersededId - The other decision involved in the dispute
 */
function resolve(keepId, action, supersededId) {
  if (action === 'keep') {
    db.run(
      "UPDATE decisions SET status = 'active', disputes_id = NULL WHERE id = ?",
      [keepId]
    );
    db.run(
      "UPDATE decisions SET status = 'superseded', supersedes_id = ?, disputes_id = NULL WHERE id = ?",
      [keepId, supersededId]
    );
  } else if (action === 'merge') {
    db.run(
      "UPDATE decisions SET status = 'active', disputes_id = NULL WHERE id = ?",
      [keepId]
    );
    db.run(
      "UPDATE decisions SET status = 'active', disputes_id = NULL WHERE id = ?",
      [supersededId]
    );
  }
  db.persist();
}

// ── Query helpers ───────────────────────────────────────────────────

/**
 * Return all disputed decisions, each paired with its dispute partner.
 *
 * @returns {Array<{decision: object, partner: object|null}>}
 */
function getDisputed() {
  const disputed = db.query(
    "SELECT * FROM decisions WHERE status = 'disputed'"
  );
  return disputed.map(d => ({
    decision: d,
    partner: d.disputes_id
      ? db.get("SELECT * FROM decisions WHERE id = ?", [d.disputes_id])
      : null,
  }));
}

module.exports = {
  detectConflict,
  markDisputed,
  resolve,
  getDisputed,
  // Exported for testing
  jaccardSimilarity,
  tokenize,
};

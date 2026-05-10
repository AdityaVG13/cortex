// 32-bit FNV-1a hash, used to derive a stable palette seed from cluster
// centroid bytes. Returns an unsigned 32-bit integer.
export function fnv1a32(bytes) {
  let hash = 2166136261;
  if (!bytes) return hash >>> 0;
  if (typeof bytes === "string") {
    for (let i = 0; i < bytes.length; i += 1) {
      hash ^= bytes.charCodeAt(i);
      hash = Math.imul(hash, 16777619);
    }
  } else {
    const len = bytes.length || bytes.byteLength || 0;
    const view = bytes.buffer ? new Uint8Array(bytes.buffer, bytes.byteOffset || 0, len) : bytes;
    for (let i = 0; i < len; i += 1) {
      hash ^= view[i] & 0xff;
      hash = Math.imul(hash, 16777619);
    }
  }
  return hash >>> 0;
}

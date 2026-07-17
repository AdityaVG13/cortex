import * as THREE from "three";
import { bezierArcPoints } from "./util/bezierArc.js";
import { createActivationTexture, createPulseMaterial } from "./PulseShader.js";
import { riseDecay } from "./util/easing.js";
const POOL_SIZE = 64, SEGMENTS = 16, VERTS_PER_BEAM = SEGMENTS + 1, RISE_MS = 80,
  TAU_MS = 280, DEFAULT_LIFE_MS = 600, _from = new THREE.Vector3(), _to = new THREE.Vector3();
function createBeams({ scene }) { const totalVerts = POOL_SIZE * VERTS_PER_BEAM,
    positions = new Float32Array(totalVerts * 3), progresses = new Float32Array(totalVerts),
    beamIds = new Float32Array(totalVerts), colors = new Float32Array(totalVerts * 3), indices = new Uint16Array(POOL_SIZE * SEGMENTS * 2);
  for (let beam = 0; beam < POOL_SIZE; beam += 1) { for (let s = 0; s <= SEGMENTS; s += 1) { const v = beam * VERTS_PER_BEAM + s;
      ((progresses[v] = s / SEGMENTS), (beamIds[v] = beam));
    }
    for (let s = 0; s < SEGMENTS; s += 1) { const i = (beam * SEGMENTS + s) * 2;
      ((indices[i] = beam * VERTS_PER_BEAM + s), (indices[i + 1] = beam * VERTS_PER_BEAM + s + 1));
    }
  }
  const geometry = new THREE.BufferGeometry();
  (geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3)), geometry.setAttribute("aProgress", new THREE.BufferAttribute(progresses, 1)),
    geometry.setAttribute("aBeamId", new THREE.BufferAttribute(beamIds, 1)), geometry.setAttribute("aColor", new THREE.BufferAttribute(colors, 3)),
    geometry.setIndex(new THREE.BufferAttribute(indices, 1)));
  const { texture, data } = createActivationTexture(POOL_SIZE), material = createPulseMaterial({ activationTexture: texture, beamCount: POOL_SIZE,
    }), mesh = new THREE.LineSegments(geometry, material);
  ((mesh.frustumCulled = !1), (mesh.renderOrder = 1), (mesh.name = "brain-v2-beams"), scene.add(mesh));
  const slots = Array.from({ length: POOL_SIZE }, () => ({ active: !1, startTime: 0, lifeMs: DEFAULT_LIFE_MS, }));
  function findSlot(now) { for (let i = 0; i < POOL_SIZE; i += 1) if (!slots[i].active) return i;
    let oldestIdx = 0, oldestTime = 1 / 0;
    for (let i = 0; i < POOL_SIZE; i += 1)
      slots[i].startTime < oldestTime && ((oldestTime = slots[i].startTime), (oldestIdx = i));
    return oldestIdx;
  }
  function fire({ from, to, color = "#22d3ee", life = DEFAULT_LIFE_MS, now = performance.now() }) { if (!from || !to) return -1;
    (_from.set(from.x, from.y, from.z), _to.set(to.x, to.y, to.z));
    const arc = bezierArcPoints(_from, _to, SEGMENTS, 0.18), slot = findSlot(now), baseVert = slot * VERTS_PER_BEAM, c = new THREE.Color(color);
    for (let i = 0; i < arc.length; i += 1) { const v = baseVert + i;
      ((positions[v * 3 + 0] = arc[i].x), (positions[v * 3 + 1] = arc[i].y), (positions[v * 3 + 2] = arc[i].z), (colors[v * 3 + 0] = c.r),
        (colors[v * 3 + 1] = c.g), (colors[v * 3 + 2] = c.b));
    }
    return ( (geometry.attributes.position.needsUpdate = !0), (geometry.attributes.aColor.needsUpdate = !0), (slots[slot].active = !0),
      (slots[slot].startTime = now), (slots[slot].lifeMs = life), (data[slot] = 0), (texture.needsUpdate = !0), slot );
  }
  function tick(now = performance.now()) { let dirty = !1;
    for (let i = 0; i < POOL_SIZE; i += 1) { const slot = slots[i];
      if (!slot.active) { data[i] !== 0 && ((data[i] = 0), (dirty = !0));
        continue;
      }
      const t = now - slot.startTime;
      if (t >= slot.lifeMs) { ((slot.active = !1), (data[i] = 0), (dirty = !0));
        continue;
      }
      const value = riseDecay(t, RISE_MS, TAU_MS);
      ((data[i] = Math.min(1, value)), (dirty = !0));
    }
    (dirty && (texture.needsUpdate = !0), (material.uniforms.uTime.value = (now * 0.001) % 1e3));
  }
  function activeCount() { return slots.reduce((n, s) => n + (s.active ? 1 : 0), 0);
  }
  function dispose() { (scene.remove(mesh), geometry.dispose(), material.dispose(), texture.dispose());
  }
  return { mesh, fire, tick, activeCount, dispose };
}
export { createBeams };

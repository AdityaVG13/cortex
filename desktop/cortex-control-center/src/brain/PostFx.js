import * as THREE from "three";
import { BloomEffect, EffectComposer, EffectPass, RenderPass } from "postprocessing";

const BLOOM_INTENSITY = 0.85;
const BLOOM_THRESHOLD = 0.18;
const BLOOM_SMOOTHING = 0.4;

const DEGRADE_DISABLE_MS = 33.3;
const DEGRADE_REENABLE_MS = 22;
const DEGRADE_SAMPLE_WINDOW_MS = 1000;
const DEGRADE_REENABLE_SUSTAIN_MS = 3000;

function median(values) {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[mid - 1] + sorted[mid]) / 2 : sorted[mid];
}

export function attachBloom(graph, { onAutoDegrade } = {}) {
  if (!graph) return null;
  const renderer = typeof graph.renderer === "function" ? graph.renderer() : null;
  const scene = typeof graph.scene === "function" ? graph.scene() : null;
  const camera = typeof graph.camera === "function" ? graph.camera() : null;
  const existingComposer = typeof graph.postProcessingComposer === "function" ? graph.postProcessingComposer() : null;

  if (!renderer || !scene || !camera) return null;

  if (typeof renderer.toneMapping !== "undefined") {
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.0;
  }

  let composer = existingComposer;
  let ownsComposer = false;
  if (!composer) {
    composer = new EffectComposer(renderer);
    composer.addPass(new RenderPass(scene, camera));
    ownsComposer = true;
  }

  const bloom = new BloomEffect({
    intensity: BLOOM_INTENSITY,
    luminanceThreshold: BLOOM_THRESHOLD,
    luminanceSmoothing: BLOOM_SMOOTHING,
    mipmapBlur: true,
  });

  const bloomPass = new EffectPass(camera, bloom);
  bloomPass.enabled = true;
  composer.addPass(bloomPass);

  let lastTimestamp = performance.now();
  let frameSamples = [];
  let bloomEnabled = true;
  let belowReenableSince = 0;

  function frame(now) {
    const dt = now - lastTimestamp;
    lastTimestamp = now;
    frameSamples.push({ t: now, dt });
    while (frameSamples.length && now - frameSamples[0].t > DEGRADE_SAMPLE_WINDOW_MS) {
      frameSamples.shift();
    }
    const med = median(frameSamples.map(s => s.dt));

    if (bloomEnabled && med >= DEGRADE_DISABLE_MS) {
      bloomEnabled = false;
      bloomPass.enabled = false;
      onAutoDegrade?.(true);
    } else if (!bloomEnabled) {
      if (med <= DEGRADE_REENABLE_MS) {
        if (!belowReenableSince) belowReenableSince = now;
        if (now - belowReenableSince >= DEGRADE_REENABLE_SUSTAIN_MS) {
          bloomEnabled = true;
          bloomPass.enabled = true;
          belowReenableSince = 0;
          onAutoDegrade?.(false);
        }
      } else {
        belowReenableSince = 0;
      }
    }
    rafHandle = requestAnimationFrame(frame);
  }
  let rafHandle = requestAnimationFrame(frame);

  return {
    bloom,
    bloomPass,
    composer,
    refreshSelection: () => {},
    setIntensity: (value) => { bloom.intensity = value; },
    isEnabled: () => bloomEnabled,
    dispose: () => {
      cancelAnimationFrame(rafHandle);
      try {
        composer.removePass(bloomPass);
        bloomPass.dispose?.();
        bloom.dispose?.();
      } catch {
        // best-effort cleanup
      }
      if (ownsComposer) composer.dispose?.();
    },
  };
}

export function refreshBloomSelection() {
  /* no-op: BloomEffect uses a luminance threshold, not a Selection */
}

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

const BACKGROUND = "#040812";
const CAMERA_FOV = 55;
const CAMERA_NEAR = 1;
const CAMERA_FAR = 2000;
const CAMERA_INITIAL = { x: 0, y: 60, z: 380 };

export function createScene({ container, width, height }) {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(BACKGROUND);

  const camera = new THREE.PerspectiveCamera(
    CAMERA_FOV,
    width / Math.max(height, 1),
    CAMERA_NEAR,
    CAMERA_FAR,
  );
  camera.position.set(CAMERA_INITIAL.x, CAMERA_INITIAL.y, CAMERA_INITIAL.z);
  camera.lookAt(0, 0, 0);

  const renderer = new THREE.WebGLRenderer({
    antialias: true,
    alpha: false,
    powerPreference: "high-performance",
  });
  renderer.setPixelRatio(window.devicePixelRatio || 1);
  renderer.setSize(width, height);
  renderer.toneMapping = THREE.LinearToneMapping;
  renderer.toneMappingExposure = 1.0;
  container.appendChild(renderer.domElement);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.target.set(0, 0, 0);
  controls.enableDamping = false;
  controls.zoomSpeed = 0.7;
  controls.minDistance = 60;
  controls.maxDistance = 800;
  controls.update();

  const ticks = new Set();
  let rafHandle = null;
  let disposed = false;
  const startTime = performance.now();

  function frame() {
    if (disposed) return;
    const now = performance.now();
    const t = (now - startTime) * 0.001;
    controls.update();
    for (const fn of ticks) {
      try {
        fn(t, now);
      } catch (err) {
        // Tick errors must not abort the loop; log once.
        // eslint-disable-next-line no-console
        console.error("[brain-v2] tick error", err);
      }
    }
    renderer.render(scene, camera);
    rafHandle = requestAnimationFrame(frame);
  }

  rafHandle = requestAnimationFrame(frame);

  function resize(nextWidth, nextHeight) {
    if (disposed) return;
    camera.aspect = nextWidth / Math.max(nextHeight, 1);
    camera.updateProjectionMatrix();
    renderer.setSize(nextWidth, nextHeight);
  }

  function dispose() {
    disposed = true;
    if (rafHandle) cancelAnimationFrame(rafHandle);
    ticks.clear();
    controls.dispose();
    renderer.dispose();
    if (renderer.domElement.parentNode) {
      renderer.domElement.parentNode.removeChild(renderer.domElement);
    }
  }

  return {
    scene,
    camera,
    renderer,
    controls,
    registerTick: (fn) => {
      ticks.add(fn);
      return () => ticks.delete(fn);
    },
    resize,
    dispose,
  };
}

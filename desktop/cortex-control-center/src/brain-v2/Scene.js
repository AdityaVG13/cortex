import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
const BACKGROUND = "#040812", CAMERA_FOV = 55, CAMERA_NEAR = 1, CAMERA_FAR = 2e3, CAMERA_INITIAL = { x: 0, y: 0, z: 380 };
function createScene({ container, width, height, animated = !0, pixelRatio = window.devicePixelRatio || 1 }) { const scene = new THREE.Scene();
  scene.background = new THREE.Color(BACKGROUND);
  const camera = new THREE.PerspectiveCamera(CAMERA_FOV, width / Math.max(height, 1), CAMERA_NEAR, CAMERA_FAR);
  (camera.position.set(CAMERA_INITIAL.x, CAMERA_INITIAL.y, CAMERA_INITIAL.z), camera.lookAt(0, 0, 0));
  const renderer = new THREE.WebGLRenderer({ antialias: !0, alpha: !1, powerPreference: "high-performance", });
  (renderer.setPixelRatio(pixelRatio), renderer.setSize(width, height), (renderer.toneMapping = THREE.LinearToneMapping), (renderer.toneMappingExposure = 1),
    container.appendChild(renderer.domElement));
  const controls = new OrbitControls(camera, renderer.domElement);
  (controls.target.set(0, 0, 0), (controls.enableDamping = !1), (controls.zoomSpeed = 0.7), (controls.minDistance = 60),
    (controls.maxDistance = 800), (controls.enablePan = !1), (controls.minPolarAngle = Math.PI * 0.35), (controls.maxPolarAngle = Math.PI * 0.65),
    controls.update());
  const ticks = new Set();
  let rafHandle = null, disposed = !1;
  const startTime = performance.now();
  function renderFrame() { if (disposed) return;
    const now = performance.now(), t = (now - startTime) * 0.001;
    controls.update();
    for (const fn of ticks)
      try { fn(t, now);
      } catch (err) { console.error("[brain-v2] tick error", err);
      }
    renderer.render(scene, camera);
  }
  function frame() { (renderFrame(), (rafHandle = animated && !disposed ? requestAnimationFrame(frame) : null));
  }
  function requestFrame() { disposed || animated || rafHandle || (rafHandle = requestAnimationFrame(frame));
  }
  (controls.addEventListener("change", requestFrame), animated ? (rafHandle = requestAnimationFrame(frame)) : requestFrame());
  function resize(nextWidth, nextHeight) { disposed || ((camera.aspect = nextWidth / Math.max(nextHeight, 1)), camera.updateProjectionMatrix(),
      renderer.setSize(nextWidth, nextHeight), requestFrame());
  }
  function dispose() { ((disposed = !0), rafHandle && cancelAnimationFrame(rafHandle), ticks.clear(),
      controls.removeEventListener("change", requestFrame), controls.dispose(),
      renderer.dispose(), renderer.domElement.parentNode && renderer.domElement.parentNode.removeChild(renderer.domElement));
  }
  return { scene, camera, renderer, controls, registerTick: (fn) => (ticks.add(fn), requestFrame(), () => ticks.delete(fn)), resize, requestFrame, dispose, };
}
export { createScene };

import * as THREE from "three";
import { easeInOutCubic } from "./util/easing.js";

const AUTO_ROTATE_RATE = 0.04;
const AUTO_RESUME_MS = 8_000;
const SPOTLIGHT_PULL = 0.15;
const SPOTLIGHT_DURATION_MS = 1_200;

export function createCamera({ camera, controls }) {
  let lastInteractionAt = 0;
  let prevTime = performance.now();
  let spotlightActive = false;
  let spotlightStart = 0;
  const cameraStart = new THREE.Vector3();
  const cameraEnd = new THREE.Vector3();
  const targetStart = new THREE.Vector3();
  const targetEnd = new THREE.Vector3();
  const _tmpCam = new THREE.Vector3();
  const _tmpTgt = new THREE.Vector3();

  function pauseAutoRotate() {
    lastInteractionAt = performance.now();
  }

  function spotlight(satelliteWorldPos) {
    if (!satelliteWorldPos) return;
    spotlightActive = true;
    spotlightStart = performance.now();
    cameraStart.copy(camera.position);
    targetStart.copy(controls.target);
    // Pull camera 15% closer along the camera→satellite vector — never re-center.
    cameraEnd.copy(satelliteWorldPos).sub(cameraStart).multiplyScalar(SPOTLIGHT_PULL).add(cameraStart);
    // Bias target toward the satellite by 15% — gentle, retains origin orientation.
    targetEnd.copy(satelliteWorldPos).multiplyScalar(SPOTLIGHT_PULL);
  }

  function tick(now = performance.now()) {
    const dt = (now - prevTime) * 0.001;
    prevTime = now;

    if (spotlightActive) {
      const elapsed = now - spotlightStart;
      if (elapsed >= SPOTLIGHT_DURATION_MS) {
        camera.position.copy(cameraEnd);
        controls.target.copy(targetEnd);
        camera.lookAt(controls.target);
        spotlightActive = false;
        // Don't bounce target back — leaving the camera focused on the
        // selection feels more "Jarvis examining" than snapping to origin.
        // Auto-rotate continues from the new pivot if the user idles.
      } else {
        const t = easeInOutCubic(elapsed / SPOTLIGHT_DURATION_MS);
        _tmpCam.copy(cameraStart).lerp(cameraEnd, t);
        _tmpTgt.copy(targetStart).lerp(targetEnd, t);
        camera.position.copy(_tmpCam);
        controls.target.copy(_tmpTgt);
        camera.lookAt(controls.target);
      }
      return;
    }

    const idle = now - lastInteractionAt;
    if (idle >= AUTO_RESUME_MS) {
      const angle = AUTO_ROTATE_RATE * dt;
      const cos = Math.cos(angle);
      const sin = Math.sin(angle);
      const tx = controls.target.x;
      const tz = controls.target.z;
      const cx = camera.position.x - tx;
      const cz = camera.position.z - tz;
      camera.position.x = tx + (cx * cos - cz * sin);
      camera.position.z = tz + (cx * sin + cz * cos);
      camera.lookAt(controls.target);
    }
  }

  return {
    pauseAutoRotate,
    spotlight,
    tick,
  };
}

export const CAMERA_AUTO_ROTATE_RATE = AUTO_ROTATE_RATE;
export const CAMERA_AUTO_RESUME_MS = AUTO_RESUME_MS;
export const CAMERA_SPOTLIGHT_PULL = SPOTLIGHT_PULL;
export const CAMERA_SPOTLIGHT_DURATION_MS = SPOTLIGHT_DURATION_MS;

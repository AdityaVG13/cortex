import * as THREE from "three";
import { easeOutCubic } from "./util/easing.js";

const AUTO_ROTATE_RATE = 0.04;
const AUTO_RESUME_MS = 8_000;
const SPOTLIGHT_PULL = 0.15;
const SPOTLIGHT_RAMP_MS = 800;
const SPOTLIGHT_RETURN_MS = 400;

const _spotlightTarget = new THREE.Vector3();
const _spotlightFromCamera = new THREE.Vector3();
const _spotlightFromTarget = new THREE.Vector3();

export function createCamera({ camera, controls }) {
  let lastInteractionAt = 0;
  let prevTime = performance.now();
  let spotlightActive = false;
  let spotlightStart = 0;
  const spotlightTargetPos = new THREE.Vector3();
  const spotlightCameraStart = new THREE.Vector3();
  const spotlightTargetStart = new THREE.Vector3();

  function pauseAutoRotate() {
    lastInteractionAt = performance.now();
  }

  function spotlight(satelliteWorldPos) {
    if (!satelliteWorldPos) return;
    spotlightActive = true;
    spotlightStart = performance.now();
    _spotlightTarget.set(satelliteWorldPos.x, satelliteWorldPos.y, satelliteWorldPos.z);
    spotlightTargetPos.copy(_spotlightTarget);
    spotlightCameraStart.copy(camera.position);
    spotlightTargetStart.copy(controls.target);
  }

  function tick(now = performance.now()) {
    const dt = (now - prevTime) * 0.001;
    prevTime = now;
    const idle = now - lastInteractionAt;
    if (idle >= AUTO_RESUME_MS) {
      const angle = AUTO_ROTATE_RATE * dt;
      const cos = Math.cos(angle);
      const sin = Math.sin(angle);
      const x = camera.position.x;
      const z = camera.position.z;
      camera.position.x = x * cos - z * sin;
      camera.position.z = x * sin + z * cos;
      camera.lookAt(controls.target);
    }

    if (spotlightActive) {
      const elapsed = now - spotlightStart;
      const totalMs = SPOTLIGHT_RAMP_MS + SPOTLIGHT_RETURN_MS;
      if (elapsed >= totalMs) {
        spotlightActive = false;
        return;
      }
      if (elapsed <= SPOTLIGHT_RAMP_MS) {
        const t = easeOutCubic(elapsed / SPOTLIGHT_RAMP_MS);
        _spotlightFromCamera.copy(spotlightTargetPos).sub(spotlightCameraStart).multiplyScalar(SPOTLIGHT_PULL);
        camera.position.copy(spotlightCameraStart).addScaledVector(_spotlightFromCamera, t);
        _spotlightFromTarget.copy(spotlightTargetPos).multiplyScalar(SPOTLIGHT_PULL);
        controls.target.copy(spotlightTargetStart).lerp(_spotlightFromTarget, t);
      } else {
        const t = easeOutCubic((elapsed - SPOTLIGHT_RAMP_MS) / SPOTLIGHT_RETURN_MS);
        _spotlightFromTarget.copy(spotlightTargetStart);
        controls.target.lerp(_spotlightFromTarget, t);
      }
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

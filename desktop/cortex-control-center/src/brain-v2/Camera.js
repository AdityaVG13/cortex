import * as THREE from "three";
import { easeInOutCubic } from "./util/easing.js";

const AUTO_ROTATE_RATE = 0.04;
const AUTO_RESUME_MS = 8_000;
const SPOTLIGHT_DURATION_MS = 1_200;
const RETURN_DURATION_MS = 900;

export function createCamera({ camera, controls }) {
  let lastInteractionAt = 0;
  let prevTime = performance.now();
  let easeActive = false;
  let easeStart = 0;
  let easeDuration = SPOTLIGHT_DURATION_MS;
  const cameraStart = new THREE.Vector3();
  const cameraEnd = new THREE.Vector3();
  const targetStart = new THREE.Vector3();
  const targetEnd = new THREE.Vector3();
  const _tmpCam = new THREE.Vector3();
  const _tmpTgt = new THREE.Vector3();
  const _offset = new THREE.Vector3();

  function pauseAutoRotate() {
    lastInteractionAt = performance.now();
  }

  function spotlight(satelliteWorldPos) {
    if (!satelliteWorldPos) return;
    easeActive = true;
    easeStart = performance.now();
    easeDuration = SPOTLIGHT_DURATION_MS;
    cameraStart.copy(camera.position);
    targetStart.copy(controls.target);
    // Look at the satellite without changing camera→target distance.
    // Camera shifts only by the same delta the target shifts by, so the
    // viewing distance stays constant across repeated clicks.
    targetEnd.set(satelliteWorldPos.x, satelliteWorldPos.y, satelliteWorldPos.z);
    _offset.copy(camera.position).sub(controls.target);
    cameraEnd.copy(targetEnd).add(_offset);
  }

  function returnToOrigin() {
    easeActive = true;
    easeStart = performance.now();
    easeDuration = RETURN_DURATION_MS;
    cameraStart.copy(camera.position);
    targetStart.copy(controls.target);
    targetEnd.set(0, 0, 0);
    _offset.copy(camera.position).sub(controls.target);
    cameraEnd.copy(targetEnd).add(_offset);
  }

  function tick(now = performance.now()) {
    const dt = (now - prevTime) * 0.001;
    prevTime = now;

    if (easeActive) {
      const elapsed = now - easeStart;
      if (elapsed >= easeDuration) {
        camera.position.copy(cameraEnd);
        controls.target.copy(targetEnd);
        camera.lookAt(controls.target);
        easeActive = false;
      } else {
        const t = easeInOutCubic(elapsed / easeDuration);
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
    returnToOrigin,
    tick,
  };
}

export const CAMERA_AUTO_ROTATE_RATE = AUTO_ROTATE_RATE;
export const CAMERA_AUTO_RESUME_MS = AUTO_RESUME_MS;
export const CAMERA_SPOTLIGHT_DURATION_MS = SPOTLIGHT_DURATION_MS;
export const CAMERA_RETURN_DURATION_MS = RETURN_DURATION_MS;

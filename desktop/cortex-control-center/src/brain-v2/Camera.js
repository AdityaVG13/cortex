import * as THREE from "three";
import { easeInOutCubic } from "./util/easing.js";
const AUTO_ROTATE_RATE = 0.04, AUTO_RESUME_MS = 8e3, SPOTLIGHT_DURATION_MS = 1200, RETURN_DURATION_MS = 900;
function createCamera({ camera, controls, autoRotate = !0 }) { let lastInteractionAt = 0, prevTime = performance.now(), easeActive = !1,
    easeStart = 0, easeDuration = SPOTLIGHT_DURATION_MS;
  const cameraStart = new THREE.Vector3(), cameraEnd = new THREE.Vector3(), targetStart = new THREE.Vector3(), targetEnd = new THREE.Vector3(),
    _tmpCam = new THREE.Vector3(), _tmpTgt = new THREE.Vector3(), _offset = new THREE.Vector3();
  function pauseAutoRotate() { lastInteractionAt = performance.now();
  }
  function spotlight(satelliteWorldPos) { if (satelliteWorldPos) { if ( (targetEnd.set(satelliteWorldPos.x, satelliteWorldPos.y, satelliteWorldPos.z),
        _offset.copy(camera.position).sub(controls.target), cameraEnd.copy(targetEnd).add(_offset), !autoRotate)
      ) { (camera.position.copy(cameraEnd), controls.target.copy(targetEnd), camera.lookAt(controls.target));
        return;
      }
      ((easeActive = !0), (easeStart = performance.now()), (easeDuration = SPOTLIGHT_DURATION_MS), cameraStart.copy(camera.position),
        targetStart.copy(controls.target));
    }
  }
  function returnToOrigin() { if ( (targetEnd.set(0, 0, 0), _offset.copy(camera.position).sub(controls.target),
      cameraEnd.copy(targetEnd).add(_offset), !autoRotate)
    ) { (camera.position.copy(cameraEnd), controls.target.copy(targetEnd), camera.lookAt(controls.target));
      return;
    }
    ((easeActive = !0), (easeStart = performance.now()), (easeDuration = RETURN_DURATION_MS), cameraStart.copy(camera.position),
      targetStart.copy(controls.target));
  }
  function tick(now = performance.now()) { const dt = (now - prevTime) * 0.001;
    if (((prevTime = now), easeActive)) { const elapsed = now - easeStart;
      if (elapsed >= easeDuration)
        (camera.position.copy(cameraEnd), controls.target.copy(targetEnd), camera.lookAt(controls.target), (easeActive = !1));
      else { const t = easeInOutCubic(elapsed / easeDuration);
        (_tmpCam.copy(cameraStart).lerp(cameraEnd, t), _tmpTgt.copy(targetStart).lerp(targetEnd, t),
          camera.position.copy(_tmpCam), controls.target.copy(_tmpTgt), camera.lookAt(controls.target));
      }
      return;
    }
    const idle = now - lastInteractionAt;
    if (autoRotate && idle >= AUTO_RESUME_MS) { const angle = AUTO_ROTATE_RATE * dt, cos = Math.cos(angle), sin = Math.sin(angle),
        tx = controls.target.x, tz = controls.target.z, cx = camera.position.x - tx, cz = camera.position.z - tz;
      ((camera.position.x = tx + (cx * cos - cz * sin)), (camera.position.z = tz + (cx * sin + cz * cos)), camera.lookAt(controls.target));
    }
  }
  return { pauseAutoRotate, spotlight, returnToOrigin, tick };
}
const CAMERA_SPOTLIGHT_DURATION_MS = SPOTLIGHT_DURATION_MS;
export { CAMERA_SPOTLIGHT_DURATION_MS, createCamera };

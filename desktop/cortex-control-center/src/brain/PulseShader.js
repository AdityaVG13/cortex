import * as THREE from "three";

const VERTEX_SHADER = /* glsl */ `
attribute float aProgress;
attribute float aEdgeId;
uniform sampler2D uActivation;
uniform float uActivationCount;
varying float vProgress;
varying float vActivation;

void main() {
  vProgress = aProgress;
  float u = (aEdgeId + 0.5) / max(uActivationCount, 1.0);
  vActivation = texture2D(uActivation, vec2(u, 0.5)).r;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

const FRAGMENT_SHADER = /* glsl */ `
precision mediump float;
uniform vec3 uBaseColor;
uniform vec3 uPulseColor;
uniform float uTime;
uniform float uHeadSpeed;
uniform float uHeadWidth;
uniform float uTrailWidth;
uniform float uBaseOpacity;
varying float vProgress;
varying float vActivation;

void main() {
  float head = mod(uTime * uHeadSpeed, 1.0);
  float lead = smoothstep(head - uHeadWidth, head, vProgress);
  float fade = 1.0 - smoothstep(head, head + uTrailWidth, vProgress);
  float pulseShape = lead * fade;

  vec3 base = uBaseColor;
  vec3 pulse = uPulseColor;

  float intensity = uBaseOpacity + (vActivation * 0.85) + (pulseShape * vActivation * 1.4);
  vec3 color = mix(base, pulse, clamp(pulseShape * vActivation, 0.0, 1.0));

  gl_FragColor = vec4(color * intensity, clamp(intensity, 0.0, 1.0));
}
`;

export function createActivationTexture(edgeCount) {
  const size = Math.max(edgeCount, 1);
  const data = new Float32Array(size);
  const texture = new THREE.DataTexture(data, size, 1, THREE.RedFormat, THREE.FloatType);
  texture.needsUpdate = true;
  texture.minFilter = THREE.NearestFilter;
  texture.magFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  return { texture, data };
}

export function createPulseMaterial({
  baseColor = "#22d3ee",
  pulseColor = "#f8fbff",
  activationTexture,
  activationCount = 1,
  baseOpacity = 0.18,
  headSpeed = 0.6,
  headWidth = 0.06,
  trailWidth = 0.18,
} = {}) {
  const material = new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uActivation: { value: activationTexture },
      uActivationCount: { value: activationCount },
      uBaseColor: { value: new THREE.Color(baseColor) },
      uPulseColor: { value: new THREE.Color(pulseColor) },
      uHeadSpeed: { value: headSpeed },
      uHeadWidth: { value: headWidth },
      uTrailWidth: { value: trailWidth },
      uBaseOpacity: { value: baseOpacity },
    },
    vertexShader: VERTEX_SHADER,
    fragmentShader: FRAGMENT_SHADER,
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  return material;
}

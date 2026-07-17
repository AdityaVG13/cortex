import*as THREE from"three";const VERTEX=`
attribute float aProgress;
attribute float aBeamId;
attribute vec3 aColor;
uniform sampler2D uActivation;
uniform float uBeamCount;
varying float vProgress;
varying float vActivation;
varying vec3 vColor;

void main() {
  vProgress = aProgress;
  vColor = aColor;
  float u = (aBeamId + 0.5) / max(uBeamCount, 1.0);
  vActivation = texture2D(uActivation, vec2(u, 0.5)).r;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`,FRAGMENT=`
precision mediump float;
uniform float uTime;
uniform float uHeadSpeed;
uniform float uHeadWidth;
uniform float uTrailWidth;
uniform float uBaseOpacity;
varying float vProgress;
varying float vActivation;
varying vec3 vColor;

void main() {
  float head = mod(uTime * uHeadSpeed, 1.0);
  float lead = smoothstep(head - uHeadWidth, head, vProgress);
  float trail = 1.0 - smoothstep(head, head + uTrailWidth, vProgress);
  float pulse = lead * trail;

  vec3 white = vec3(1.0);
  vec3 color = mix(vColor, white, clamp(pulse, 0.0, 1.0));
  float intensity = (uBaseOpacity + vActivation * 0.85 + pulse * vActivation * 1.4);
  gl_FragColor = vec4(color * intensity, clamp(intensity, 0.0, 1.0));
}
`;function createActivationTexture(slotCount){const size=Math.max(slotCount,1),data=new Float32Array(size),texture=new THREE.DataTexture(data,size,1,THREE.RedFormat,THREE.FloatType);return texture.needsUpdate=!0,texture.minFilter=THREE.NearestFilter,texture.magFilter=THREE.NearestFilter,texture.generateMipmaps=!1,{texture,data}}function createPulseMaterial({activationTexture,beamCount,baseOpacity=0,headSpeed=.6,headWidth=.1,trailWidth=.22}={}){return new THREE.ShaderMaterial({uniforms:{uTime:{value:0},uActivation:{value:activationTexture},uBeamCount:{value:beamCount},uHeadSpeed:{value:headSpeed},uHeadWidth:{value:headWidth},uTrailWidth:{value:trailWidth},uBaseOpacity:{value:baseOpacity}},vertexShader:VERTEX,fragmentShader:FRAGMENT,transparent:!0,blending:THREE.AdditiveBlending,depthWrite:!1})}export{createActivationTexture,createPulseMaterial};

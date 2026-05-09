import { useEffect, useRef, useState } from "react";
import { createScene } from "./Scene.js";
import { createCore, tickCore, disposeCore } from "./Core.js";

export function BrainV2({ active = true }) {
  const containerRef = useRef(null);
  const sceneRef = useRef(null);
  const coreRef = useRef(null);
  const [dimensions, setDimensions] = useState({
    width: Math.max(window.innerWidth - 260, 400),
    height: Math.max(window.innerHeight - 20, 300),
  });

  useEffect(() => {
    if (!active) return undefined;
    function onResize() {
      setDimensions({
        width: Math.max(window.innerWidth - 260, 400),
        height: Math.max(window.innerHeight - 20, 300),
      });
    }
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [active]);

  useEffect(() => {
    if (!active || !containerRef.current) return undefined;
    const sceneHandle = createScene({
      container: containerRef.current,
      width: dimensions.width,
      height: dimensions.height,
    });
    sceneRef.current = sceneHandle;

    const core = createCore();
    coreRef.current = core;
    sceneHandle.scene.add(core);

    const unregister = sceneHandle.registerTick((t, now) => {
      tickCore(core, t, now);
    });

    return () => {
      unregister();
      if (coreRef.current) {
        sceneHandle.scene.remove(coreRef.current);
        disposeCore(coreRef.current);
        coreRef.current = null;
      }
      sceneHandle.dispose();
      sceneRef.current = null;
    };
  }, [active]);

  useEffect(() => {
    if (!sceneRef.current) return;
    sceneRef.current.resize(dimensions.width, dimensions.height);
  }, [dimensions.width, dimensions.height]);

  return (
    <div
      ref={containerRef}
      className="brain-v2-container"
      style={{
        position: "relative",
        width: dimensions.width,
        height: dimensions.height,
        background: "#040812",
        overflow: "hidden",
      }}
    />
  );
}

export default BrainV2;

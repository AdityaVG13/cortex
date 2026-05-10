import { paletteForCluster, DECISION_COLOR, LOOSE_COLOR } from "./ClusterPalette.js";

const ORIGIN = { x: 0, y: 0, z: 0 };

function colorForSlot(slot) {
  if (!slot) return LOOSE_COLOR.getStyle();
  if (slot.tier === "decision") return DECISION_COLOR.getStyle();
  if (slot.tier === "loose") return LOOSE_COLOR.getStyle();
  if (slot.coldStart) return LOOSE_COLOR.getStyle();
  if (slot.centroidKey) return paletteForCluster(slot.centroidKey).color.getStyle();
  return LOOSE_COLOR.getStyle();
}

export function createEventDispatcher({
  satellites,
  beams,
  core,
  pulseCoreHalo,
  onTickerEntry,
  onSpotlight,
}) {
  function findSlot(rawId) {
    if (!satellites || !rawId) return null;
    const direct = satellites.getSlotById(rawId);
    if (direct) return direct;
    if (rawId.startsWith("memory-")) {
      return satellites.getSlotById(`loose-${rawId.slice(7)}`)
        || satellites.getSlotById(`cold-cluster-${rawId.slice(7)}`);
    }
    if (rawId.startsWith("decision-")) {
      return satellites.getSlotById(`decision-${rawId.slice(9)}`);
    }
    if (rawId.startsWith("crystal-")) {
      return satellites.getSlotById(`cluster-${rawId.slice(8)}`);
    }
    return null;
  }

  function dispatch(event) {
    if (!event || typeof event !== "object") return;
    const type = event.type;
    switch (type) {
      case "consolidation_started":
        if (typeof pulseCoreHalo === "function") pulseCoreHalo();
        onTickerEntry?.("consolidation_started");
        break;
      case "member_added": {
        const member = findSlot(event.member_id);
        const cluster = findSlot(`crystal-${event.cluster_id}`);
        if (member && cluster) {
          beams?.fire({ from: member, to: cluster, color: colorForSlot(cluster) });
          satellites?.pulseSlot(member.id);
        }
        onTickerEntry?.(`member_added · cluster ${event.cluster_id}`);
        break;
      }
      case "cluster_finalized": {
        const cluster = findSlot(`crystal-${event.cluster_id}`);
        if (cluster) {
          satellites?.pulseSlot(cluster.id);
          onSpotlight?.(cluster);
        }
        onTickerEntry?.(`cluster_finalized · ${event.member_count || "?"} members`);
        break;
      }
      case "link_inferred": {
        const a = findSlot(event.a);
        const b = findSlot(event.b);
        if (a && b) {
          beams?.fire({ from: a, to: b, color: colorForSlot(a) });
        }
        onTickerEntry?.("link_inferred");
        break;
      }
      case "recall": {
        const ids = Array.isArray(event.node_ids) ? event.node_ids : [];
        for (const id of ids) {
          const slot = findSlot(id);
          if (slot) {
            satellites?.pulseSlot(slot.id);
            beams?.fire({ from: slot, to: ORIGIN, color: colorForSlot(slot), life: 500 });
          }
        }
        if (typeof pulseCoreHalo === "function") pulseCoreHalo();
        onTickerEntry?.(`recall · ${ids.length} nodes`);
        break;
      }
      default:
        break;
    }
  }

  function dispatchFake(slotId) {
    const slot = satellites?.getSlotById(slotId);
    if (!slot) return;
    beams?.fire({ from: slot, to: ORIGIN, color: colorForSlot(slot), life: 500 });
    satellites?.pulseSlot(slotId);
  }

  return { dispatch, dispatchFake };
}

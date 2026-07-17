import {
  paletteForCluster,
  DECISION_COLOR,
  LOOSE_COLOR,
} from "./ClusterPalette.js";
const ORIGIN = { x: 0, y: 0, z: 0 };
function colorForSlot(slot) {
  return slot
    ? slot.tier === "decision"
      ? DECISION_COLOR.getStyle()
      : slot.tier === "loose"
        ? LOOSE_COLOR.getStyle()
        : slot.coldStart
          ? LOOSE_COLOR.getStyle()
          : slot.centroidKey
            ? paletteForCluster(slot.centroidKey).color.getStyle()
            : LOOSE_COLOR.getStyle()
    : LOOSE_COLOR.getStyle();
}
function createEventDispatcher({
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
    return (
      direct ||
      (rawId.startsWith("memory-")
        ? satellites.getSlotById(`loose-${rawId.slice(7)}`) ||
          satellites.getSlotById(`cold-cluster-${rawId.slice(7)}`)
        : rawId.startsWith("decision-")
          ? satellites.getSlotById(`decision-${rawId.slice(9)}`)
          : rawId.startsWith("crystal-")
            ? satellites.getSlotById(`cluster-${rawId.slice(8)}`)
            : null)
    );
  }
  function dispatch(event) {
    if (!event || typeof event != "object") return;
    switch (event.type) {
      case "consolidation_started":
        (typeof pulseCoreHalo == "function" && pulseCoreHalo(),
          onTickerEntry?.("consolidation_started"));
        break;
      case "member_added": {
        const member = findSlot(event.member_id),
          cluster = findSlot(`crystal-${event.cluster_id}`);
        (member &&
          cluster &&
          (beams?.fire({
            from: member,
            to: cluster,
            color: colorForSlot(cluster),
          }),
          satellites?.pulseSlot(member.id),
          onSpotlight?.(member)),
          onTickerEntry?.(`member_added \xB7 cluster ${event.cluster_id}`));
        break;
      }
      case "cluster_finalized": {
        const cluster = findSlot(`crystal-${event.cluster_id}`);
        (cluster && (satellites?.pulseSlot(cluster.id), onSpotlight?.(cluster)),
          onTickerEntry?.(
            `cluster_finalized \xB7 ${event.member_count || "?"} members`,
          ));
        break;
      }
      case "link_inferred": {
        const a = findSlot(event.a),
          b = findSlot(event.b);
        (a &&
          b &&
          (beams?.fire({ from: a, to: b, color: colorForSlot(a) }),
          onSpotlight?.(a)),
          onTickerEntry?.("link_inferred"));
        break;
      }
      case "recall": {
        const ids = Array.isArray(event.node_ids) ? event.node_ids : [];
        let firstSlot = null;
        for (const id of ids) {
          const slot = findSlot(id);
          slot &&
            (firstSlot || (firstSlot = slot),
            satellites?.pulseSlot(slot.id),
            beams?.fire({
              from: slot,
              to: ORIGIN,
              color: colorForSlot(slot),
              life: 500,
            }));
        }
        (firstSlot && onSpotlight?.(firstSlot),
          typeof pulseCoreHalo == "function" && pulseCoreHalo(),
          onTickerEntry?.(`recall \xB7 ${ids.length} nodes`));
        break;
      }
      default:
        break;
    }
  }
  function dispatchFake(slotId) {
    const slot = satellites?.getSlotById(slotId);
    slot &&
      (beams?.fire({
        from: slot,
        to: ORIGIN,
        color: colorForSlot(slot),
        life: 500,
      }),
      satellites?.pulseSlot(slotId));
  }
  return { dispatch, dispatchFake };
}
export { createEventDispatcher };

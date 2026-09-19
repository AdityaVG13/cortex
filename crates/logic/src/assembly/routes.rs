use super::LearningEvent;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct RouteScore {
    pub target: String,
    pub utility: f64,
    pub mass: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteEdge {
    pub cue: String,
    pub target: String,
    pub positive: f64,
    pub negative: f64,
}

/// Rebuild cue→assembly edges. One training unit contributes at most once per
/// cue/target. Conflicting labels inside a unit are discarded, not voted.
pub fn route_edges(events: &[LearningEvent], now: i64) -> Result<Vec<RouteEdge>, String> {
    if now < 0 {
        return Err("invalid_read_time".into());
    }
    let mut units: BTreeMap<(String, String, String), Vec<&LearningEvent>> = BTreeMap::new();
    for event in events {
        event.validate()?;
        if event.observed_at > now || !event.kind.counts_as_reward() || event.reward == 0 {
            continue;
        }
        for cue in &event.cues {
            units
                .entry((
                    event.training_unit.clone(),
                    cue.clone(),
                    event.target.clone(),
                ))
                .or_default()
                .push(event);
        }
    }
    let mut edges: BTreeMap<(String, String), (f64, f64)> = BTreeMap::new();
    for ((_, cue, target), group) in units {
        let rewards: BTreeSet<i8> = group.iter().map(|event| event.reward).collect();
        if rewards.len() != 1 {
            continue;
        }
        let chosen = group
            .into_iter()
            .min_by_key(|event| {
                (
                    event.observed_at,
                    event.origin.as_str(),
                    event.origin_event_id.as_str(),
                )
            })
            .expect("non-empty group");
        let slot = edges.entry((cue, target)).or_insert((0.0, 0.0));
        if chosen.reward > 0 {
            slot.0 += 1.0;
        } else {
            slot.1 += 1.0;
        }
    }
    Ok(edges
        .into_iter()
        .map(|((cue, target), (positive, negative))| RouteEdge {
            cue,
            target,
            positive,
            negative,
        })
        .collect())
}

pub fn rank_routes(
    edges: &[RouteEdge],
    cues: &[String],
    allowed: &[String],
    min_mass: f64,
) -> Vec<RouteScore> {
    let query: BTreeSet<_> = cues.iter().cloned().collect();
    let allowed: BTreeSet<_> = allowed.iter().cloned().collect();
    let mut scores: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for edge in edges {
        if !query.contains(&edge.cue) || !allowed.contains(&edge.target) {
            continue;
        }
        let slot = scores.entry(edge.target.clone()).or_insert((0.0, 0.0));
        slot.0 += edge.positive;
        slot.1 += edge.negative;
    }
    let mut ranked: Vec<_> = scores
        .into_iter()
        .filter_map(|(target, (positive, negative))| {
            let mass = positive + negative;
            if mass < min_mass {
                return None;
            }
            Some(RouteScore {
                target,
                utility: (positive - negative) / (2.0 + mass),
                mass,
            })
        })
        .collect();
    ranked.sort_by(|left, right| {
        right
            .utility
            .total_cmp(&left.utility)
            .then(left.target.cmp(&right.target))
    });
    ranked
}

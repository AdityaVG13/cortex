use super::*;
use rusqlite::Connection;
pub fn prune_crystal_member_embeddings(_conn: &Connection) -> usize {
    0
}

pub fn prune_orphan_cluster_members(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
) -> usize {
    let mut count = 0usize;
    count += exec_counted(
        conn,
        failures,
        "prune_orphan_cluster_members DELETE cluster_members (orphan memory)",
        "DELETE FROM cluster_members \
         WHERE target_type = 'memory' \
           AND NOT EXISTS (SELECT 1 FROM memories WHERE memories.id = cluster_members.target_id)",
        [],
    );
    count += exec_counted(
        conn,
        failures,
        "prune_orphan_cluster_members DELETE cluster_members (orphan decision)",
        "DELETE FROM cluster_members \
         WHERE target_type = 'decision' \
           AND NOT EXISTS (SELECT 1 FROM decisions WHERE decisions.id = cluster_members.target_id)",
        [],
    );
    count += exec_counted(
        conn,
        failures,
        "prune_orphan_cluster_members DELETE cluster_members (unknown target_type)",
        "DELETE FROM cluster_members \
         WHERE target_type NOT IN ('memory', 'decision')",
        [],
    );
    count += exec_counted(
        conn,
        failures,
        "prune_orphan_cluster_members DELETE cluster_members (orphan cluster_id)",
        "DELETE FROM cluster_members \
         WHERE NOT EXISTS (SELECT 1 FROM memory_clusters WHERE memory_clusters.id = cluster_members.cluster_id)",
        [],
    );
    count
}

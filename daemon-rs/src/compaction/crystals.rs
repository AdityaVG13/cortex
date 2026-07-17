use rusqlite::Connection;pub(crate)fn prune_crystal_member_embeddings(conn:&Connection)->usize{let mut count=0usize;count+=conn.
execute(
"DELETE FROM embeddings WHERE target_type = 'memory' AND target_id IN (\
                SELECT target_id FROM cluster_members WHERE target_type = 'memory'\
             )"
,[],).unwrap_or(0);count+=conn.execute(
"DELETE FROM embeddings WHERE target_type = 'decision' AND target_id IN (\
                SELECT target_id FROM cluster_members WHERE target_type = 'decision'\
             )"
,[],).unwrap_or(0);count}pub(crate)fn prune_orphan_cluster_members(conn:&Connection)->usize{let mut count=0usize;count+=conn.
execute(
"DELETE FROM cluster_members \
             WHERE target_type = 'memory' \
               AND NOT EXISTS (SELECT 1 FROM memories WHERE memories.id = cluster_members.target_id)"
,[],).unwrap_or(0);count+=conn.execute(
"DELETE FROM cluster_members \
             WHERE target_type = 'decision' \
               AND NOT EXISTS (SELECT 1 FROM decisions WHERE decisions.id = cluster_members.target_id)"
,[],).unwrap_or(0);count+=conn.execute(
"DELETE FROM cluster_members \
             WHERE target_type NOT IN ('memory', 'decision')",[],).unwrap_or(0);count+=conn.execute
(
"DELETE FROM cluster_members \
             WHERE NOT EXISTS (SELECT 1 FROM memory_clusters WHERE memory_clusters.id = cluster_members.cluster_id)"
,[],).unwrap_or(0);count}

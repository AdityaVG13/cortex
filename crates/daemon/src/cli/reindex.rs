use super::common::or_die;
use crate::auth;
use crate::crystallize;
use crate::db;
use crate::state;
use serde_json::json;

pub fn run_reindex_cli(paths: &auth::CortexPaths, json_output: bool) {
    let conn = or_die(db::open(&paths.db), "Error: failed to open database for reindex: ");
    or_die(db::configure(&conn), "Error: failed to configure database for reindex: ");
    let memories_base = db::count_or_zero(&conn, "SELECT COUNT(*) FROM memories WHERE status = 'active'");
    let decisions_base = db::count_or_zero(&conn, "SELECT COUNT(*) FROM decisions WHERE status = 'active'");
    or_die(db::reindex_fts(&conn), "Error: failed to rebuild FTS indexes: ");
    let memories_fts = db::count_or_zero(&conn, "SELECT COUNT(*) FROM memories_fts");
    let decisions_fts = db::count_or_zero(&conn, "SELECT COUNT(*) FROM decisions_fts");
    if json_output {
        println!(
            "{}",
            json!({"reindexed":true,"counts":{"memories_base":memories_base,"memories_fts":memories_fts,"decisions_base":decisions_base,"decisions_fts":decisions_fts,}})
        );
        return;
    }
    println!("Reindex complete");
    println!("memories: base={memories_base}, fts={memories_fts}");
    println!("decisions: base={decisions_base}, fts={decisions_fts}");
}
pub async fn run_recrystallize_cli(cx: &asupersync::Cx, paths: &auth::CortexPaths, json_output: bool) {
    let (state, _shutdown_rx) = or_die(state::initialize(paths, false), "Error: failed to initialize state for recrystallize: ");
    let result_payload = {
        let conn = or_die(state.db.lock(cx).await, "[cortex] ");
        let (crystals_before, members_before, embeddings_before) = cluster_counts(&conn);
        let removed_embeddings = 0usize;
        // `cluster_members` has no FK to `memory_clusters`. Deleting only
        // clusters leaves member rows that still exclude those targets from
        // `scan_candidates`, so recrystallize would no-op on already-clustered
        // rows. Clear members first; crystals have no cascade. A failed DELETE
        // must not continue into a pass that reports `recrystallized: true`.
        let tx = or_die(conn.unchecked_transaction(), "Error: failed to begin recrystallize transaction: ");
        let removed_members = or_die(tx.execute("DELETE FROM cluster_members", []), "Error: failed to clear cluster_members: ");
        let removed_crystals = or_die(tx.execute("DELETE FROM memory_clusters", []), "Error: failed to clear memory_clusters: ");
        or_die(tx.commit(), "Error: failed to commit recrystallize clear: ");
        let brain_sender = Some(state.brain_firing.clone());
        let pass = or_die(crystallize::run_crystallize_pass_with_brain(cx, &conn, state.default_owner_id, &brain_sender), "[cortex] ");
        let (crystals_after, members_after, embeddings_after) = cluster_counts(&conn);
        json!({"recrystallized":true,"owner_filter":state.default_owner_id,"removed":{"crystals":removed_crystals,"members":removed_members,"embeddings":removed_embeddings},"before":{"crystals":crystals_before,"members":members_before,"embeddings":embeddings_before},"pass":{"clusters_found":pass.clusters_found,"crystals_created":pass.crystals_created,"crystals_updated":pass.crystals_updated,"entries_consolidated":pass.entries_consolidated},"after":{"crystals":crystals_after,"members":members_after,"embeddings":embeddings_after}})
    };
    if json_output {
        println!("{result_payload}");
        return;
    }
    println!("Recrystallize complete");
    println!(
        "removed: crystals={}, members={}, crystal_embeddings={}",
        result_payload["removed"]["crystals"].as_i64().unwrap_or_default(),
        result_payload["removed"]["members"].as_i64().unwrap_or_default(),
        result_payload["removed"]["embeddings"].as_i64().unwrap_or_default()
    );
    println!(
        "pass: clusters_found={}, created={}, updated={}, consolidated={}",
        result_payload["pass"]["clusters_found"].as_u64().unwrap_or_default(),
        result_payload["pass"]["crystals_created"].as_u64().unwrap_or_default(),
        result_payload["pass"]["crystals_updated"].as_u64().unwrap_or_default(),
        result_payload["pass"]["entries_consolidated"].as_u64().unwrap_or_default()
    );
    println!(
        "after: crystals={}, members={}, crystal_embeddings={}",
        result_payload["after"]["crystals"].as_i64().unwrap_or_default(),
        result_payload["after"]["members"].as_i64().unwrap_or_default(),
        result_payload["after"]["embeddings"].as_i64().unwrap_or_default()
    );
}

fn cluster_counts(conn: &rusqlite::Connection) -> (i64, i64, i64) {
    (
        db::count_or_zero(conn, "SELECT COUNT(*) FROM memory_clusters"),
        db::count_or_zero(conn, "SELECT COUNT(*) FROM cluster_members"),
        db::count_or_zero(conn, "SELECT COUNT(*) FROM embeddings WHERE target_type = 'crystal'"),
    )
}

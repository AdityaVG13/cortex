use crate::auth;
use crate::crystallize;
use crate::db;
use crate::state;
use serde_json::json;
pub(crate) fn run_reindex_cli(paths: &auth::CortexPaths, json_output: bool) {
    let conn = match db::open(&paths.db) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("Error: failed to open database for reindex: {err}");
            std::process::exit(1);
        }
    };
    if let Err(err) = db::configure(&conn) {
        eprintln!("Error: failed to configure database for reindex: {err}");
        std::process::exit(1);
    }
    let memories_base = conn
        .query_row("SELECT COUNT(*) FROM memories WHERE status = 'active'", [], |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    let decisions_base = conn
        .query_row("SELECT COUNT(*) FROM decisions WHERE status = 'active'", [], |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    if let Err(err) = db::reindex_fts(&conn) {
        eprintln!("Error: failed to rebuild FTS indexes: {err}");
        std::process::exit(1);
    }
    let memories_fts = conn.query_row("SELECT COUNT(*) FROM memories_fts", [], |row| row.get::<_, i64>(0)).unwrap_or(0);
    let decisions_fts = conn.query_row("SELECT COUNT(*) FROM decisions_fts", [], |row| row.get::<_, i64>(0)).unwrap_or(0);
    if json_output {
        println!(
            "{}",
            json!({"reindexed":
true,"counts":{"memories_base":memories_base,"memories_fts":memories_fts,"decisions_base":decisions_base,"decisions_fts":
decisions_fts,}})
        );
        return;
    }
    println!("Reindex complete");
    println!("memories: base={memories_base}, fts={memories_fts}");
    println!("decisions: base={decisions_base}, fts={decisions_fts}");
}
pub(crate) async fn run_recrystallize_cli(paths: &auth::CortexPaths, json_output: bool) {
    let (state, _shutdown_rx) = match state::initialize(paths, false) {
        Ok(initialized) => initialized,
        Err(err) => {
            eprintln!("Error: failed to initialize state for recrystallize: {err}");
            std::process::exit(1);
        }
    };
    let result_payload = {
        let conn = state.db.lock().await;
        let crystals_before: i64 = conn.query_row("SELECT COUNT(*) FROM memory_clusters", [], |row| row.get::<_, i64>(0)).unwrap_or(0);
        let members_before: i64 = conn.query_row("SELECT COUNT(*) FROM cluster_members", [], |row| row.get::<_, i64>(0)).unwrap_or(0);
        let embeddings_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM embeddings WHERE target_type = 'crystal'", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0);
        let removed_embeddings = conn.execute("DELETE FROM embeddings WHERE target_type = 'crystal'", []).unwrap_or(0);
        let removed_crystals = conn.execute("DELETE FROM memory_clusters", []).unwrap_or(0);
        let brain_sender = Some(state.brain_firing.clone());
        let pass =
            crystallize::run_crystallize_pass_with_brain(&conn, state.embedding_engine.as_deref(), state.default_owner_id, &brain_sender);
        let crystals_after: i64 = conn.query_row("SELECT COUNT(*) FROM memory_clusters", [], |row| row.get::<_, i64>(0)).unwrap_or(0);
        let members_after: i64 = conn.query_row("SELECT COUNT(*) FROM cluster_members", [], |row| row.get::<_, i64>(0)).unwrap_or(0);
        let embeddings_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM embeddings WHERE target_type = 'crystal'", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0);
        json!({"recrystallized":true,"owner_filter":state.default_owner_id,"removed":{"crystals":removed_crystals,"members":
members_before,"embeddings":removed_embeddings},"before":{"crystals":crystals_before,"members":members_before,"embeddings":
embeddings_before},"pass":{"clusters_found":pass.clusters_found,"crystals_created":pass.crystals_created,"crystals_updated":pass.
crystals_updated,"entries_consolidated":pass.entries_consolidated},"after":{"crystals":crystals_after,"members":members_after,
"embeddings":embeddings_after}})
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

use cortex_kernel::runtime::{BootInput, CortexRuntime, LensInput};
use cortex_kernel::traces::{list_versions, rollback_to};
use cortex_tests::support::{run_with_cx, solo_state};

const AGENT: &str = "history-agent";
const DECISION_A: &str = "We chose sqlite WAL journaling HISTORYALPHA for the ledger";
const DECISION_B: &str = "Billing exports move to parquet snapshots HISTORYBETA nightly";
const DECISION_C: &str = "Frontend bundle splits vendor chunks HISTORYGAMMA aggressively";

async fn recall_excerpts(cx: &asupersync::Cx, runtime: &CortexRuntime, query: &str) -> Vec<String> {
    let result = runtime.lens(cx, LensInput { query: query.into(), budget: 600, k: 10,
        agent: AGENT.into(), ..Default::default() }).await.expect("lens");
    result["results"].as_array().expect("results").iter().map(|r| r["excerpt"].as_str().unwrap().to_owned()).collect()
}

// HTTP status/envelope assertions retired; rollback's typed count/head and visibility are pinned.
#[test]
fn rollback_hides_later_store_from_recall_and_boot() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let a = runtime.deposit(&cx, "a", DECISION_A, AGENT, None).await.unwrap();
        let version_a = a.entry["versionId"].as_i64().unwrap();
        let b = runtime.deposit(&cx, "b", DECISION_B, AGENT, None).await.unwrap();
        let version_b = b.entry["versionId"].as_i64().unwrap();
        assert!(version_b > version_a);
        let before = recall_excerpts(&cx, &runtime, "HISTORYBETA parquet exports").await;
        assert!(before.iter().any(|e| e == DECISION_B));
        {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            let (orphaned, head) = rollback_to(&conn, version_a).expect("rollback");
            assert_eq!(head, version_a);
            assert_eq!(orphaned, 1);
        }
        let after = recall_excerpts(&cx, &runtime, "HISTORYBETA parquet exports").await;
        assert!(after.iter().all(|e| !e.contains("HISTORYBETA")));
        let alpha = recall_excerpts(&cx, &runtime, "HISTORYALPHA sqlite ledger").await;
        assert!(alpha.iter().any(|e| e == DECISION_A));
        let boot = runtime.boot(&cx, BootInput { agent: AGENT.into(), max_tokens: 600, owner_id: None, ..Default::default() }).await.unwrap();
        assert!(!boot.boot_prompt.contains("HISTORYBETA"));
        let c = runtime.deposit(&cx, "c", DECISION_C, AGENT, None).await.unwrap();
        let version_c = c.entry["versionId"].as_i64().unwrap();
        assert!(version_c > version_b);
        let gamma = recall_excerpts(&cx, &runtime, "HISTORYGAMMA vendor chunks").await;
        assert!(gamma.iter().any(|e| e == DECISION_C));
        let beta_again = recall_excerpts(&cx, &runtime, "HISTORYBETA parquet exports").await;
        assert!(beta_again.iter().all(|e| !e.contains("HISTORYBETA")));
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let versions = list_versions(&conn, 50);
        let row_b = versions.iter().find(|v| v["id"] == version_b).expect("B retained in log");
        assert_eq!(row_b["status"], "orphaned");
    });
}

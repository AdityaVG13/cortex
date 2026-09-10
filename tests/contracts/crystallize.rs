use cortex_kernel::crystallize::{list_crystals, run_crystallize_pass_with_brain};
use cortex_kernel::runtime::CortexRuntime;
use cortex_tests::support::{run_with_cx, solo_state};

const DUP_A: &str = "payments service cluster postgres migration rollout requires idempotency keys deploy window checklist";
const DUP_B: &str = "payments service cluster postgres migration rollout blocked pending schema review sign off";
const DUP_C: &str = "payments service cluster postgres migration rollout staged behind feature flag ramp plan";
const DUP_D: &str = "payments service cluster postgres migration rollout owner assigned oncall rotation handles paging";
const UNRELATED_E: &str = "The quarterly financial audit report needs review by the compliance team for regulatory approval";
const UNRELATED_F: &str = "Recipe for sourdough bread requires flour water salt and long fermentation overnight";

// Listener readiness and HTTP status assertions retired; exercise the actual Jaccard pass.
#[test]
fn crystallize_groups_duplicates_and_is_idempotent() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        for text in [DUP_A, DUP_B, DUP_C, DUP_D, UNRELATED_E, UNRELATED_F] {
            let stored = runtime.deposit(&cx, text, text, "crystallize-test", None).await.expect("deposit");
            assert!(stored.target_id.is_some());
        }
        let conn = runtime.state().db.lock(&cx).await.expect("db capability");
        let first = run_crystallize_pass_with_brain(&cx, &conn, None, &None).expect("first pass");
        assert!(first.clusters_found >= 1);
        assert_eq!(first.crystals_created, first.clusters_found);
        assert_eq!(first.crystals_updated, 0);
        assert!(first.entries_consolidated >= 4);
        let crystals = list_crystals(&conn);
        assert_eq!(crystals.len(), first.clusters_found);
        let big = crystals.iter().find(|c| c["members"].as_u64().unwrap() >= 4).expect("four-member crystal");
        assert!(!big["label"].as_str().unwrap().is_empty());
        assert!([DUP_A, DUP_B, DUP_C, DUP_D].contains(&big["text"].as_str().unwrap()));
        let mut stmt = conn.prepare("SELECT source FROM cluster_members WHERE cluster_id = ?1 ORDER BY source").expect("members");
        let sources: Vec<String> = stmt.query_map([big["id"].as_i64().unwrap()], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        let mut expected = vec![DUP_A, DUP_B, DUP_C, DUP_D];
        expected.sort();
        assert_eq!(sources, expected);
        for unrelated in [UNRELATED_E, UNRELATED_F] {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM cluster_members WHERE source = ?1", [unrelated], |r| r.get(0)).unwrap();
            assert_eq!(count, 0);
        }
        let second = run_crystallize_pass_with_brain(&cx, &conn, None, &None).expect("second pass");
        assert_eq!(second.clusters_found, 0);
        assert_eq!(second.crystals_created, 0);
        assert_eq!(second.entries_consolidated, 0);
        assert_eq!(list_crystals(&conn).len(), crystals.len());
    });
}

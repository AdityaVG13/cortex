//! Strong-match qualification: a hard anchor is identity only inside its
//! namespace. A path is hard within its repository/source root, a ticket
//! within its issuer, and a bare alias (`main`, `src`) is never identity
//! across two roots. Absent namespace evidence on either side keeps the match
//! hard: qualification demotes on *disagreement*, never on ignorance, so an
//! exact single-source report stays retrievable.

use super::anchors::AnchorKind;
use super::query::QueryAnchor;
use std::collections::BTreeSet;

/// Namespace evidence carried by the query side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnchorScope {
    /// Longest common normalized prefix of the task's explicit paths.
    pub repo_root: Option<String>,
    /// Issuer hosts named by the query (tracker/url hosts).
    pub issuer_hosts: BTreeSet<String>,
}

/// Namespace evidence carried by one candidate row (its own projected anchors).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowNamespace {
    pub paths: Vec<String>,
    pub hosts: Vec<String>,
}

const HARD: u8 = 3;
const DEMOTED: u8 = 2;

fn segments(value: &str) -> Vec<&str> {
    value.split('/').filter(|s| !s.is_empty()).collect()
}

impl AnchorScope {
    pub fn from_query(task_paths: &[String], anchors: &[QueryAnchor]) -> Self {
        let normalized: Vec<Vec<String>> = task_paths
            .iter()
            .map(|p| super::anchors::normalize_anchor_value(AnchorKind::Path, p))
            .filter(|p| !p.is_empty())
            .map(|p| segments(&p).iter().map(|s| s.to_string()).collect())
            .collect();
        let repo_root = common_root(&normalized);
        let issuer_hosts = anchors
            .iter()
            .filter(|a| a.kind == AnchorKind::UrlHost)
            .map(|a| a.value.clone())
            .collect();
        Self {
            repo_root,
            issuer_hosts,
        }
    }
}

/// Common leading segments of every task path minus the file/dir tail; needs
/// at least two shared segments to count as a root.
fn common_root(paths: &[Vec<String>]) -> Option<String> {
    let first = paths.first()?;
    let mut n = first.len();
    for p in paths.iter().skip(1) {
        n = n.min(
            first
                .iter()
                .zip(p.iter())
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    if paths.len() == 1 {
        n = n.saturating_sub(1);
    }
    if n < 2 {
        return None;
    }
    Some(first[..n].join("/"))
}

/// A row path disagrees with the repo root when it is root-qualified in the
/// same filesystem namespace (shares the first segment) yet does not sit
/// under the root.
fn path_disagrees(row_path: &str, repo_root: &str) -> bool {
    let root = segments(repo_root);
    let row = segments(row_path);
    if row.len() < root.len() || row.first() != root.first() {
        return false;
    }
    !(row_path == repo_root || row_path.starts_with(&format!("{repo_root}/")))
}

/// Demote matched anchors whose namespace disagrees with the query scope.
/// Returns the qualified anchors and the reasons for every demotion.
pub fn qualify_matches(
    matched: &[QueryAnchor],
    row: &RowNamespace,
    scope: &AnchorScope,
) -> (Vec<QueryAnchor>, Vec<String>) {
    let mut out = Vec::with_capacity(matched.len());
    let mut reasons = Vec::new();
    for anchor in matched {
        let mut qualified = anchor.clone();
        if anchor.specificity >= HARD {
            match anchor.kind {
                AnchorKind::Path => {
                    if segments(&anchor.value).len() < 2 {
                        qualified.specificity = DEMOTED;
                        reasons.push(format!("path_alias:{}", anchor.value));
                    } else if let Some(root) = scope.repo_root.as_deref() {
                        let foreign = row.paths.iter().filter(|p| path_disagrees(p, root)).count();
                        let local = row
                            .paths
                            .iter()
                            .filter(|p| {
                                !path_disagrees(p, root)
                                    && segments(p).len() >= segments(root).len()
                            })
                            .count();
                        if foreign > 0 && local == 0 {
                            qualified.specificity = DEMOTED;
                            reasons.push(format!("path_outside_root:{}", anchor.value));
                        }
                    }
                }
                AnchorKind::Ticket => {
                    if !scope.issuer_hosts.is_empty()
                        && !row.hosts.is_empty()
                        && !row.hosts.iter().any(|h| scope.issuer_hosts.contains(h))
                    {
                        qualified.specificity = DEMOTED;
                        reasons.push(format!("ticket_issuer_mismatch:{}", anchor.value));
                    }
                }
                _ => {}
            }
        }
        out.push(qualified);
    }
    (out, reasons)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qa(kind: AnchorKind, v: &str) -> QueryAnchor {
        QueryAnchor {
            kind,
            value: v.into(),
            specificity: 3,
        }
    }

    #[test]
    fn repo_root_is_the_shared_prefix_of_task_paths() {
        let s = AnchorScope::from_query(
            &[
                "/Users/x/repoa/src/lib.rs".into(),
                "/Users/x/repoa/tests/a.rs".into(),
            ],
            &[],
        );
        assert_eq!(s.repo_root.as_deref(), Some("users/x/repoa"));
        let single = AnchorScope::from_query(&["/Users/x/repoa/src".into()], &[]);
        assert_eq!(single.repo_root.as_deref(), Some("users/x/repoa"));
        assert_eq!(
            AnchorScope::from_query(&["src".into()], &[]).repo_root,
            None
        );
    }

    #[test]
    fn relative_path_inside_a_foreign_root_is_not_identity() {
        let scope = AnchorScope::from_query(&["/Users/x/repoa/src".into()], &[]);
        let foreign = RowNamespace {
            paths: vec!["users/x/repob/src/lib.rs".into(), "src/lib.rs".into()],
            hosts: vec![],
        };
        let (q, why) = qualify_matches(&[qa(AnchorKind::Path, "src/lib.rs")], &foreign, &scope);
        assert_eq!(q[0].specificity, 2, "{why:?}");
        let local = RowNamespace {
            paths: vec!["users/x/repoa/src/lib.rs".into(), "src/lib.rs".into()],
            hosts: vec![],
        };
        assert_eq!(
            qualify_matches(&[qa(AnchorKind::Path, "src/lib.rs")], &local, &scope).0[0].specificity,
            3
        );
        let unknown = RowNamespace {
            paths: vec!["src/lib.rs".into()],
            hosts: vec![],
        };
        assert_eq!(
            qualify_matches(&[qa(AnchorKind::Path, "src/lib.rs")], &unknown, &scope).0[0]
                .specificity,
            3,
            "ignorance never demotes"
        );
    }

    #[test]
    fn bare_aliases_and_foreign_issuers_are_demoted() {
        let scope = AnchorScope {
            repo_root: None,
            issuer_hosts: ["jira.example.com".to_string()].into_iter().collect(),
        };
        let row = RowNamespace {
            paths: vec![],
            hosts: vec!["linear.app".into()],
        };
        let (q, why) = qualify_matches(
            &[
                qa(AnchorKind::Path, "main"),
                qa(AnchorKind::Ticket, "pay-77"),
            ],
            &row,
            &scope,
        );
        assert_eq!(q[0].specificity, 2);
        assert_eq!(q[1].specificity, 2);
        assert_eq!(why.len(), 2);
        let same = RowNamespace {
            paths: vec![],
            hosts: vec!["jira.example.com".into()],
        };
        assert_eq!(
            qualify_matches(&[qa(AnchorKind::Ticket, "pay-77")], &same, &scope).0[0].specificity,
            3
        );
    }
}

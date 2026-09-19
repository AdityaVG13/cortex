use super::*;

impl ReadSnapshot for MemorySnapshot {
    fn frontier(&self) -> &Frontier {
        &self.frontier
    }
    fn get(&self, refs: &[LogicalId]) -> Result<Vec<Row>, StoreSpiError> {
        Ok(refs
            .iter()
            .filter_map(|r| {
                self.rows
                    .iter()
                    .find(|(k, _)| k == &r.canonical())
                    .map(|(_, row)| row.clone())
            })
            .collect())
    }
    fn scan(
        &self,
        predicate: &Predicate,
        continuation: Option<&str>,
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        let after = continuation.unwrap_or("");
        let mut rows = Vec::new();
        let mut examined = 0u64;
        let mut bytes = 0u64;
        let mut truncated = false;
        if limits.rows == 0 {
            return Ok(Page::covered(rows, self.frontier.clone(), true, 0, None));
        }
        for (_key, row) in self.rows.iter().filter(|(k, _)| k.as_str() > after) {
            examined += 1;
            if !predicate_ok(predicate, row) {
                continue;
            }
            if rows.len() as u32 >= limits.rows {
                truncated = true;
                break;
            }
            let text_len = row.body["text"]
                .as_str()
                .map(|t| t.len() as u64)
                .unwrap_or(0);
            if bytes.saturating_add(text_len) > limits.bytes && limits.bytes > 0 {
                if rows.is_empty() {
                    continue;
                }
                truncated = true;
                break;
            }
            bytes = bytes.saturating_add(text_len);
            rows.push(row.clone());
        }
        let continuation = if truncated {
            rows.last().map(|r| r.id.canonical())
        } else {
            None
        };
        Ok(Page::covered(
            rows,
            self.frontier.clone(),
            !truncated,
            examined,
            continuation,
        ))
    }
    fn candidates(
        &self,
        profile: &CandidateProfile,
        keys: &[String],
        limits: ScanLimits,
    ) -> Result<Page, StoreSpiError> {
        if *profile != CandidateProfile::ExactLexical {
            return Err(StoreSpiError::Unavailable(format!(
                "memory oracle provides only the exact profile, not {profile:?}"
            )));
        }
        let needles: Vec<String> = keys
            .iter()
            .map(|k| k.to_ascii_lowercase())
            .filter(|k| !k.is_empty())
            .collect();
        let mut rows: Vec<Row> = self
            .rows
            .iter()
            .filter(|(_, r)| r.body["status"] == "active")
            .filter(|(_, r)| {
                !needles.is_empty()
                    && needles.iter().all(|n| {
                        r.body["text"]
                            .as_str()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .contains(n.as_str())
                    })
            })
            .map(|(_, r)| r.clone())
            .collect();
        rows.sort_by(|a, b| a.id.canonical().cmp(&b.id.canonical()));
        let exhausted = rows.len() as u32 <= limits.rows;
        rows.truncate(limits.rows as usize);
        Ok(Page::covered(
            rows,
            self.frontier.clone(),
            exhausted,
            self.rows.len() as u64,
            None,
        ))
    }
}

fn predicate_ok(p: &Predicate, row: &Row) -> bool {
    match p {
        Predicate::All => true,
        Predicate::Kind(k) => match k.as_str() {
            "decision" => row.id.namespace == "decision",
            "memory" => row.id.namespace == "memory",
            other => row.kind == other,
        },
        Predicate::Eq { field, value } => row.body.get(field) == Some(value),
        Predicate::And(items) => items.iter().all(|i| predicate_ok(i, row)),
    }
}

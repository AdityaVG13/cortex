pub(super) fn compress_to_key_points(text: &str) -> String {
    let sentences: Vec<&str> = text
        .split(['.', '\n'])
        .map(|s| s.trim())
        .filter(|s| s.len() > 5)
        .collect();
    if sentences.len() <= 2 {
        return text.chars().take(300).collect();
    }
    let high_signal = [
        "must",
        "never",
        "always",
        "critical",
        "important",
        "decision",
        "fixed",
        "bug",
        "error",
        "confirmed",
        "approved",
        "rejected",
        "architecture",
        "design",
        "migration",
        "breaking",
        "security",
    ];
    let mut kept: Vec<&str> = Vec::new();
    kept.push(sentences[0]);
    for sentence in &sentences[1..] {
        let lower = sentence.to_lowercase();
        if high_signal.iter().any(|kw| lower.contains(kw)) && kept.len() < 4 {
            kept.push(sentence);
        }
    }
    let result = kept.join(". ");
    if result.len() > 300 {
        result.chars().take(300).collect::<String>() + "..."
    } else {
        result
    }
}
pub(super) fn compress_to_one_liner(text: &str) -> String {
    let first_sentence = text
        .split(['.', '\n'])
        .map(|s| s.trim())
        .find(|s| s.len() > 5)
        .unwrap_or(text);
    first_sentence.chars().take(120).collect()
}
/// Salience is not deletion authority: low score can demote operational
/// rows to the archive placement, never a durable-retention row. Time-based
/// aging uses the same durable skip.
pub fn get_display_text(text: &str, compressed_text: &Option<String>, age_tier: &str) -> String {
    match age_tier {
        "fresh" => text.to_string(),
        _ => compressed_text
            .as_ref()
            .filter(|c| !c.is_empty())
            .cloned()
            .unwrap_or_else(|| text.to_string()),
    }
}

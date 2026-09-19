use super::normalize_text;

pub fn coding_synonyms(word: &str) -> Option<&'static str> {
    const PAIRS: &[(&str, &str)] = &[
        ("abroad", "overseas"),
        ("arg", "argument"),
        ("args", "arguments"),
        ("async", "asynchronous"),
        ("attend", "attended"),
        ("attended", "attend"),
        ("auth", "authentication"),
        ("authn", "authentication"),
        ("authz", "authorization"),
        ("bool", "boolean"),
        ("bought", "buy"),
        ("buy", "bought"),
        ("cfg", "config"),
        ("char", "character"),
        ("color", "colour"),
        ("colour", "color"),
        ("config", "configuration"),
        ("conn", "connection"),
        ("coupon", "voucher"),
        ("db", "database"),
        ("dict", "dictionary"),
        ("dir", "directory"),
        ("env", "environment"),
        ("err", "error"),
        ("fn", "function"),
        ("func", "function"),
        ("gift", "present"),
        ("gray", "grey"),
        ("grey", "gray"),
        ("idx", "index"),
        ("impl", "implementation"),
        ("int", "integer"),
        ("lastname", "surname"),
        ("msg", "message"),
        ("num", "number"),
        ("obj", "object"),
        ("overseas", "abroad"),
        ("painted", "paint"),
        ("param", "parameter"),
        ("params", "parameters"),
        ("present", "gift"),
        ("repaint", "paint"),
        ("repainted", "paint"),
        ("repo", "repository"),
        ("req", "request"),
        ("res", "response"),
        ("resp", "response"),
        ("rx", "receive"),
        ("stmt", "statement"),
        ("str", "string"),
        ("surname", "lastname"),
        ("sync", "synchronous"),
        ("tmp", "temporary"),
        ("tx", "transaction"),
        ("var", "variable"),
        ("vec", "vector"),
        ("voucher", "coupon"),
        ("wall", "walls"),
        ("walls", "wall"),
    ];
    PAIRS
        .binary_search_by_key(&word, |&(key, _)| key)
        .ok()
        .map(|idx| PAIRS[idx].1)
}

pub fn query_intent_alias_terms(text: &str) -> Vec<String> {
    let lower = normalize_text(text);
    let mut aliases = Vec::new();
    if lower.contains("study abroad") {
        aliases.extend(
            ["attend", "attended", "exchange", "semester"]
                .into_iter()
                .map(str::to_string),
        );
    }
    if lower.contains("coupon") && lower.contains("creamer") {
        aliases.extend(
            ["redeem", "redeemed", "store", "grocery"]
                .into_iter()
                .map(str::to_string),
        );
    }
    if lower.contains("birthday") && (lower.contains("gift") || lower.contains("present")) {
        aliases.extend(
            ["buy", "bought", "item", "present"]
                .into_iter()
                .map(str::to_string),
        );
    }
    aliases
}

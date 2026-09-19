use rustc_hash::{FxBuildHasher, FxHashSet};

pub fn fold_jaccard_token(token: &str) -> String {
    if token.bytes().all(|byte| byte.is_ascii()) {
        if token.bytes().any(|byte| byte.is_ascii_uppercase()) {
            token.to_ascii_lowercase()
        } else {
            token.to_owned()
        }
    } else {
        token.to_lowercase()
    }
}

fn ascii_tokens_already_folded(text: &str) -> bool {
    text.bytes().all(|byte| !byte.is_ascii_uppercase())
}

fn jaccard_ratio(left_len: usize, right_len: usize, intersection: usize) -> f64 {
    if left_len == 0 && right_len == 0 {
        return 1.0;
    }
    if left_len == 0 || right_len == 0 {
        return 0.0;
    }
    let union = (left_len + right_len) as f64 - intersection as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection as f64 / union
    }
}

fn jaccard_borrowed(a: &str, b: &str) -> f64 {
    let mut left = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    for word in a.split_whitespace().filter(|word| word.len() > 1) {
        left.insert(word);
    }
    let mut right = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    for word in b.split_whitespace().filter(|word| word.len() > 1) {
        right.insert(word);
    }
    let (smaller, larger) = if left.len() <= right.len() {
        (&left, &right)
    } else {
        (&right, &left)
    };
    let intersection = smaller
        .iter()
        .filter(|token| larger.contains(*token))
        .count();
    jaccard_ratio(left.len(), right.len(), intersection)
}

pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    if ascii_tokens_already_folded(a) && ascii_tokens_already_folded(b) {
        jaccard_borrowed(a, b)
    } else {
        jaccard_similarity_token_sets(&jaccard_token_set(a), &jaccard_token_set(b))
    }
}

pub(super) fn fill_jaccard_tokens(text: &str, tokens: &mut FxHashSet<String>) {
    tokens.clear();
    for word in text.split_whitespace().filter(|word| word.len() > 1) {
        tokens.insert(fold_jaccard_token(word));
    }
}

pub fn jaccard_token_set(text: &str) -> FxHashSet<String> {
    let mut tokens = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    fill_jaccard_tokens(text, &mut tokens);
    tokens
}

pub fn jaccard_similarity_token_sets(left: &FxHashSet<String>, right: &FxHashSet<String>) -> f64 {
    let (smaller, larger) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let intersection = smaller
        .iter()
        .filter(|token| larger.contains(*token))
        .count();
    jaccard_ratio(left.len(), right.len(), intersection)
}

const NEGATION_TOKENS: &[&str] = &[
    "not",
    "never",
    "no",
    "without",
    "avoid",
    "dont",
    "cant",
    "wont",
    "cannot",
    "disable",
    "disabled",
    "forbid",
    "forbidden",
    "against",
];

pub(super) fn semantic_tokens(text: &str) -> FxHashSet<String> {
    let mut tokens = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    // Apostrophes are not token breaks: splitting on non-alnum turned
    // "don't"/"can't" into "don"/"can" + "t", so contracted negations never
    // matched the negation lexicon and "Always X" vs "Don't X" missed CONTRADICTS.
    let normalized: String = text
        .chars()
        .filter(|ch| *ch != '\'' && *ch != '\u{2019}')
        .collect();
    for token in normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() > 1)
    {
        tokens.insert(if token.bytes().any(|byte| byte.is_ascii_uppercase()) {
            token.to_ascii_lowercase()
        } else {
            token.to_owned()
        });
    }
    tokens
}

pub(super) fn has_negation(tokens: &FxHashSet<String>) -> bool {
    NEGATION_TOKENS.iter().any(|token| tokens.contains(*token))
}

pub(super) fn strip_negation_tokens(tokens: &FxHashSet<String>) -> FxHashSet<String> {
    tokens
        .iter()
        .filter(|token| !NEGATION_TOKENS.contains(&token.as_str()))
        .cloned()
        .collect()
}

pub(super) fn has_polarity_flip(
    tokens_a: &FxHashSet<String>,
    tokens_b: &FxHashSet<String>,
) -> bool {
    const FLIP_PAIRS: &[(&str, &str)] = &[
        ("always", "never"),
        ("must", "never"),
        ("allow", "forbid"),
        ("enable", "disable"),
        ("use", "avoid"),
    ];
    FLIP_PAIRS.iter().any(|(lhs, rhs)| {
        (tokens_a.contains(*lhs) && tokens_b.contains(*rhs))
            || (tokens_a.contains(*rhs) && tokens_b.contains(*lhs))
    })
}

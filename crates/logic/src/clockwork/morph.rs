//! Deterministic morphology for Clock-Quorum Recall.
//!
//! Inspired by Porter (1980) / Krovetz (1993) stemming, but closed and
//! suffix-table based: no dictionary, no network, no model. Used to treat
//! cache/caching/cached as one lexical handle without cosine neighbors.

fn alnum_lower(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn is_cons(c: char) -> bool {
    matches!(c, 'b'..='d' | 'f'..='h' | 'j'..='n' | 'p'..='t' | 'v'..='z')
}

fn undouble(base: &mut String) {
    let chars: Vec<char> = base.chars().collect();
    if chars.len() >= 4 {
        let last = chars[chars.len() - 1];
        let prev = chars[chars.len() - 2];
        if last == prev && is_cons(last) {
            base.pop();
        }
    }
}

/// Compact technical stem: plurals, -ing/-ed, -ation/-ate, then terminal e.
pub fn morph_stem(token: &str) -> String {
    let mut w = alnum_lower(token);
    if w.len() < 4 {
        return w;
    }
    if w.ends_with("ies") && w.len() > 5 {
        w.truncate(w.len() - 3);
        w.push('y');
    } else if (w.ends_with("ses")
        || w.ends_with("xes")
        || w.ends_with("zes")
        || w.ends_with("ches")
        || w.ends_with("shes"))
        && w.len() > 5
    {
        w.truncate(w.len() - 2);
    } else if w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && w.len() > 4 {
        w.pop();
    }
    if w.ends_with("ing") && w.len() >= 6 {
        w.truncate(w.len() - 3);
        undouble(&mut w);
    } else if w.ends_with("ed") && w.len() >= 5 {
        w.truncate(w.len() - 2);
        undouble(&mut w);
    }
    for suffix in [
        "ational", "ization", "iveness", "ation", "izing", "ator", "ment", "ness", "ity", "ate",
        "als", "al",
    ] {
        if w.ends_with(suffix) && w.len() > suffix.len() + 3 {
            w.truncate(w.len() - suffix.len());
            break;
        }
    }
    if w.ends_with('e') && w.len() >= 5 {
        w.pop();
    }
    w
}

pub fn stems_match(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    let sa = morph_stem(a);
    let sb = morph_stem(b);
    sa.len() >= 4 && sa == sb
}

/// Light inflection set for query/document expansion. Deterministic, capped.
pub fn morph_variants(token: &str) -> Vec<String> {
    let t = alnum_lower(token);
    if t.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let push = |out: &mut Vec<String>, value: String| {
        if value.len() >= 3 && !out.iter().any(|existing| existing == &value) {
            out.push(value);
        }
    };
    push(&mut out, t.clone());
    let stem = morph_stem(&t);
    if stem != t {
        push(&mut out, stem.clone());
    }
    if !t.ends_with('s') {
        push(&mut out, format!("{t}s"));
        if t.ends_with('y') && t.len() > 3 {
            let mut ies = t.clone();
            ies.pop();
            ies.push_str("ies");
            push(&mut out, ies);
        }
    }
    if t.ends_with('e') {
        let base = &t[..t.len() - 1];
        push(&mut out, format!("{base}ed"));
        push(&mut out, format!("{base}ing"));
    } else {
        push(&mut out, format!("{t}ed"));
        push(&mut out, format!("{t}ing"));
        if t.ends_with('e') {
            push(&mut out, format!("{t}d"));
        }
    }
    if t.ends_with("ing") && t.len() >= 6 {
        let base = &t[..t.len() - 3];
        push(&mut out, base.to_string());
        push(&mut out, format!("{base}e"));
    }
    if t.ends_with("ed") && t.len() >= 5 {
        let base = &t[..t.len() - 2];
        push(&mut out, base.to_string());
        push(&mut out, format!("{base}e"));
    }
    out.truncate(8);
    out
}

pub fn hay_has_lexical(hay_lower: &str, term: &str) -> bool {
    if term.is_empty() {
        return false;
    }
    let needle = term.to_ascii_lowercase();
    if hay_lower.contains(&needle) {
        return true;
    }
    hay_lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|tok| !tok.is_empty() && stems_match(tok, &needle))
}

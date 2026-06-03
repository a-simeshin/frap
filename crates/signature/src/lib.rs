use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.85;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DOMToken {
    pub tag: String,
    pub role: Option<String>,
    pub semantic_type: Option<String>,
    pub structural_class: Option<String>,
    pub depth: u8,
}

impl DOMToken {
    pub fn matches(&self, other: &DOMToken) -> bool {
        self.tag == other.tag && self.role == other.role
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub path: Vec<DOMToken>,
    pub prefix: String,
    pub stable_attrs: HashMap<String, String>,
    pub text_content: Option<String>,
    pub position_in_parent: Option<usize>,
    pub children_hash: u64,
    pub depth: u8,
}

impl Signature {
    pub fn path_string(&self) -> String {
        self.path
            .iter()
            .map(|t| format!("{}:{}", t.tag, t.role.as_deref().unwrap_or("-")))
            .collect::<Vec<_>>()
            .join(">")
    }

    pub fn tokens(&self) -> &[DOMToken] {
        &self.path
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub selector: String,
    pub signature: Signature,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealResult {
    pub healed: bool,
    pub selector: String,
    pub confidence: f64,
    pub diff: Option<String>,
    pub top_candidates: Vec<Candidate>,
    pub original_signature: Signature,
}

impl HealResult {
    pub fn success(selector: String) -> Self {
        Self {
            healed: false,
            selector,
            confidence: 1.0,
            diff: None,
            top_candidates: vec![],
            original_signature: Signature {
                path: vec![],
                prefix: String::new(),
                stable_attrs: HashMap::new(),
                text_content: None,
                position_in_parent: None,
                children_hash: 0,
                depth: 0,
            },
        }
    }

    pub fn healed(
        selector: String,
        confidence: f64,
        candidates: Vec<Candidate>,
        original: Signature,
    ) -> Self {
        Self {
            healed: true,
            selector,
            confidence,
            diff: Some(format!("Healed with confidence {:.2}", confidence)),
            top_candidates: candidates.into_iter().take(3).collect(),
            original_signature: original,
        }
    }

    pub fn failed(candidates: Vec<Candidate>, original: Signature) -> Self {
        Self {
            healed: false,
            selector: String::new(),
            confidence: 0.0,
            diff: Some("No suitable candidate found".to_string()),
            top_candidates: candidates.into_iter().take(3).collect(),
            original_signature: original,
        }
    }
}

pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let len_a = a.chars().count();
    let len_b = b.chars().count();

    if len_a == 0 {
        return len_b;
    }
    if len_b == 0 {
        return len_a;
    }

    let mut matrix = vec![vec![0; len_b + 1]; len_a + 1];

    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate() {
        *cell = j;
    }

    for (i, ca) in a.chars().enumerate() {
        for (j, cb) in b.chars().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            matrix[i + 1][j + 1] = *[
                matrix[i][j + 1] + 1,
                matrix[i + 1][j] + 1,
                matrix[i][j] + cost,
            ]
            .iter()
            .min()
            .unwrap();
        }
    }

    matrix[len_a][len_b]
}

pub fn longest_common_subsequence_len<T: Eq>(
    a: &[T],
    b: &[T],
    matcher: impl Fn(&T, &T) -> bool,
) -> usize {
    let mut dp = vec![vec![0; b.len() + 1]; a.len() + 1];

    for i in 1..=a.len() {
        for j in 1..=b.len() {
            if matcher(&a[i - 1], &b[j - 1]) {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    dp[a.len()][b.len()]
}

pub fn calculate_path_similarity(a: &str, b: &str) -> f64 {
    let distance = levenshtein_distance(a, b);
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - (distance as f64 / max_len as f64)
}

pub fn calculate_token_similarity(a: &[DOMToken], b: &[DOMToken]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let lcs_len = longest_common_subsequence_len(a, b, |t1, t2| t1.matches(t2));
    let max_len = a.len().max(b.len());
    lcs_len as f64 / max_len as f64
}

pub fn calculate_structural_similarity(a: u64, b: u64) -> f64 {
    if a == b {
        1.0
    } else if a == 0 || b == 0 {
        0.5
    } else {
        0.0
    }
}

/// Bonus when an element identifier moved between `id` and `data-id` with the same value.
/// Penalty when candidate carries a different `id` / `data-id` than the original.
pub fn calculate_identifier_conflict_penalty(original: &Signature, candidate: &Signature) -> f64 {
    let mut penalty = 0.0;

    if let Some(orig_id) = original.stable_attrs.get("id") {
        if let Some(cand_id) = candidate.stable_attrs.get("id") {
            if orig_id != cand_id {
                penalty += 0.4;
            }
        }
    }

    if let Some(orig_data_id) = original.stable_attrs.get("data-id") {
        if let Some(cand_data_id) = candidate.stable_attrs.get("data-id") {
            if orig_data_id != cand_data_id {
                penalty += 0.4;
            }
        }
    }

    penalty
}

pub fn calculate_identifier_migration_bonus(original: &Signature, candidate: &Signature) -> f64 {
    let orig_id = original.stable_attrs.get("id");
    let orig_data_id = original.stable_attrs.get("data-id");
    let cand_id = candidate.stable_attrs.get("id");
    let cand_data_id = candidate.stable_attrs.get("data-id");

    if let Some(value) = orig_id {
        if cand_data_id == Some(value) {
            return 0.35;
        }
        if cand_id == Some(value) {
            return 0.1;
        }
    }

    if let Some(value) = orig_data_id {
        if cand_id == Some(value) {
            return 0.35;
        }
        if cand_data_id == Some(value) {
            return 0.1;
        }
    }

    0.0
}

pub fn calculate_attribute_bonus(original: &Signature, candidate: &Signature) -> f64 {
    let mut bonus = 0.0;

    if let (Some(orig_text), Some(cand_text)) = (&original.text_content, &candidate.text_content) {
        if orig_text == cand_text && !orig_text.is_empty() {
            bonus += 0.1;
        }
    }

    if let (Some(orig_pos), Some(cand_pos)) =
        (original.position_in_parent, candidate.position_in_parent)
    {
        if orig_pos == cand_pos {
            bonus += 0.08;
        }
    }

    bonus += calculate_identifier_migration_bonus(original, candidate);

    for (key, value) in &original.stable_attrs {
        if candidate.stable_attrs.get(key) == Some(value) {
            bonus += 0.05;
        }
    }

    if let (Some(orig_tid), Some(cand_tid)) = (
        original.stable_attrs.get("data-testid"),
        candidate.stable_attrs.get("data-testid"),
    ) {
        if orig_tid != cand_tid {
            let orig_parts: Vec<&str> = orig_tid
                .split(&['-', '_'][..])
                .filter(|p| p.len() > 2)
                .collect();
            let cand_parts: Vec<&str> = cand_tid
                .split(&['-', '_'][..])
                .filter(|p| p.len() > 2)
                .collect();
            if orig_parts
                .iter()
                .any(|p| cand_parts.iter().any(|c| c.contains(p) || p.contains(c)))
            {
                bonus += 0.15;
            }
        }
    }

    bonus
}

pub fn calculate_confidence(original: &Signature, candidate: &Signature) -> f64 {
    let path_sim = calculate_path_similarity(&original.path_string(), &candidate.path_string());
    let token_sim = calculate_token_similarity(&original.path, &candidate.path);
    let structural_sim =
        calculate_structural_similarity(original.children_hash, candidate.children_hash);
    let bonus = calculate_attribute_bonus(original, candidate);
    let penalty = calculate_identifier_conflict_penalty(original, candidate);

    (0.5 * path_sim + 0.3 * token_sim + 0.2 * structural_sim + bonus - penalty).clamp(0.0, 1.0)
}

pub fn extract_stable_attrs(attrs: &[(String, String)]) -> HashMap<String, String> {
    let stable_keys = [
        "role",
        "type",
        "placeholder",
        "aria-label",
        "aria-labelledby",
        "name",
    ];
    let mut result = HashMap::new();

    for (key, value) in attrs {
        if stable_keys.contains(&key.as_str()) && !value.is_empty() {
            result.insert(key.clone(), value.clone());
        }
    }

    result
}

/// CSS-in-JS / framework prefixes whose generated suffixes must never be used as locators.
const GENERATED_PREFIXES: [&str; 5] = ["sc-", "css-", "jss", "mui", "emotion"];

/// Minimum share of digits inside a segment for it to read as a hash token.
const HASH_DIGIT_RATIO: f64 = 0.2;
/// Minimum Shannon entropy (bits/char) for a mixed alnum segment to read as a hash token.
const HASH_ENTROPY_BITS: f64 = 2.5;
/// Shortest segment length we even bother scoring for entropy.
const HASH_MIN_LEN: usize = 4;

/// Heuristic detector for machine-generated `id` / CSS class values.
///
/// Returns `true` when the value looks generated (CSS-in-JS hash, build-time class,
/// random token) and therefore must not be used as a stable locator. Dictionary-style
/// kebab/snake identifiers (`submit-button`, `header-widget-top-menu__link`) stay `false`.
pub fn looks_like_generated(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }

    // CSS-modules / runtime-hash classes commonly start with an underscore (`_1x2y3z`).
    if value.starts_with('_') {
        return true;
    }

    let lower = value.to_ascii_lowercase();
    if GENERATED_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return true;
    }

    // Score each `-`/`_` segment: any hash-looking segment marks the whole value generated.
    value
        .split(&['-', '_'][..])
        .filter(|s| !s.is_empty())
        .any(segment_looks_hashed)
}

/// `true` when a single token reads as a random/generated hash rather than a word.
fn segment_looks_hashed(segment: &str) -> bool {
    let len = segment.chars().count();
    if len < HASH_MIN_LEN {
        return false;
    }

    let digits = segment.chars().filter(|c| c.is_ascii_digit()).count();
    let letters = segment.chars().filter(|c| c.is_ascii_alphabetic()).count();
    let has_lower = segment.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = segment.chars().any(|c| c.is_ascii_uppercase());

    // Mixed case + at least one digit is a hallmark of generated tokens (`aWaO2mb7`).
    if has_lower && has_upper && digits > 0 {
        return true;
    }

    // Digits interleaved with letters at high entropy (`x1n2j8q9`, `1a2b3c`).
    digits > 0
        && letters > 0
        && digit_ratio(segment) >= HASH_DIGIT_RATIO
        && shannon_entropy(segment) >= HASH_ENTROPY_BITS
}

/// Share of ASCII digits among all characters of `value` (0.0 for empty input).
fn digit_ratio(value: &str) -> f64 {
    let total = value.chars().count();
    if total == 0 {
        return 0.0;
    }
    let digits = value.chars().filter(|c| c.is_ascii_digit()).count();
    digits as f64 / total as f64
}

/// Shannon entropy in bits per character (0.0 for empty input).
fn shannon_entropy(value: &str) -> f64 {
    let total = value.chars().count();
    if total == 0 {
        return 0.0;
    }

    let mut counts: HashMap<char, usize> = HashMap::new();
    for c in value.chars() {
        *counts.entry(c).or_insert(0) += 1;
    }

    let total = total as f64;
    counts
        .values()
        .map(|&count| {
            let p = count as f64 / total;
            -p * p.log2()
        })
        .sum()
}

pub fn looks_like_semantic(value: &str) -> bool {
    let semantic_keywords = [
        "submit", "search", "login", "logout", "save", "cancel", "confirm", "pay", "checkout",
    ];
    let lower = value.to_lowercase();
    semantic_keywords.iter().any(|kw| lower.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("a", ""), 1);
        assert_eq!(levenshtein_distance("", "a"), 1);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
    }

    #[test]
    fn test_path_similarity() {
        let sim = calculate_path_similarity("div>button>span", "div>button>span");
        assert_eq!(sim, 1.0);

        let sim = calculate_path_similarity("div>button", "div>input");
        assert!(sim < 1.0);
        assert!(sim > 0.0);
    }

    #[test]
    fn test_token_similarity() {
        let tokens_a = vec![
            DOMToken {
                tag: "div".to_string(),
                role: None,
                semantic_type: None,
                structural_class: None,
                depth: 0,
            },
            DOMToken {
                tag: "button".to_string(),
                role: Some("submit".to_string()),
                semantic_type: None,
                structural_class: None,
                depth: 1,
            },
        ];
        let tokens_b = vec![
            DOMToken {
                tag: "div".to_string(),
                role: None,
                semantic_type: None,
                structural_class: None,
                depth: 0,
            },
            DOMToken {
                tag: "button".to_string(),
                role: Some("submit".to_string()),
                semantic_type: None,
                structural_class: None,
                depth: 1,
            },
        ];
        let sim = calculate_token_similarity(&tokens_a, &tokens_b);
        assert_eq!(sim, 1.0);
    }

    #[test]
    fn test_lcs_similarity() {
        let a = vec![1, 2, 3, 4];
        let b = vec![2, 3, 5];
        let lcs = longest_common_subsequence_len(&a, &b, |x, y| x == y);
        assert_eq!(lcs, 2);
    }

    #[test]
    fn test_identifier_migration_id_to_data_id() {
        let original = Signature {
            path: vec![DOMToken {
                tag: "li".to_string(),
                role: None,
                semantic_type: None,
                structural_class: None,
                depth: 0,
            }],
            prefix: "html:->ul:->li:-".to_string(),
            stable_attrs: [("id".to_string(), "2".to_string())].into(),
            text_content: Some("second".to_string()),
            position_in_parent: Some(1),
            children_hash: 0,
            depth: 3,
        };

        let candidate = Signature {
            path: original.path.clone(),
            prefix: original.prefix.clone(),
            stable_attrs: [("data-id".to_string(), "2".to_string())].into(),
            text_content: Some("second".to_string()),
            position_in_parent: Some(1),
            children_hash: 0,
            depth: 3,
        };

        let bonus = calculate_identifier_migration_bonus(&original, &candidate);
        assert!(bonus >= 0.35);

        let confidence = calculate_confidence(&original, &candidate);
        assert!(confidence >= 0.7);
    }

    #[test]
    fn test_confidence_calculation() {
        let original = Signature {
            path: vec![
                DOMToken {
                    tag: "div".to_string(),
                    role: Some("form".to_string()),
                    semantic_type: None,
                    structural_class: None,
                    depth: 0,
                },
                DOMToken {
                    tag: "button".to_string(),
                    role: Some("submit".to_string()),
                    semantic_type: None,
                    structural_class: None,
                    depth: 1,
                },
            ],
            prefix: "div:form>button:submit".to_string(),
            stable_attrs: [("role".to_string(), "button".to_string())].into(),
            text_content: Some("Pay".to_string()),
            position_in_parent: Some(0),
            children_hash: 12345,
            depth: 2,
        };

        let candidate = original.clone();
        let confidence = calculate_confidence(&original, &candidate);
        assert!(confidence >= 0.9);
    }

    #[test]
    fn entropy_rejects_styled_component_hash() {
        let generated = [
            "sc-a0h3m9",
            "css-1ab2cd",
            "_1x2y3z",
            "aWaO2mb7",
            "jss123",
            "MuiButton-root-1a2b3c",
            "x1n2j8q9",
        ];
        for value in generated {
            assert!(
                looks_like_generated(value),
                "expected `{value}` to be detected as generated"
            );
        }
    }

    #[test]
    fn entropy_keeps_kebab_semantic_class() {
        let semantic = [
            "header-widget-top-menu__link",
            "submit-button",
            "user-profile-card",
            "nav_item",
            "login",
            "search",
            "header",
            "gigachat-input",
        ];
        for value in semantic {
            assert!(
                !looks_like_generated(value),
                "expected `{value}` to be kept as a semantic locator"
            );
        }
    }

    #[test]
    fn entropy_empty_value_is_not_generated() {
        assert!(!looks_like_generated(""));
    }

    #[test]
    fn shannon_entropy_basics() {
        assert_eq!(shannon_entropy(""), 0.0);
        assert_eq!(shannon_entropy("aaaa"), 0.0);
        assert!((shannon_entropy("ab") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn digit_ratio_basics() {
        assert_eq!(digit_ratio(""), 0.0);
        assert_eq!(digit_ratio("abc"), 0.0);
        assert!((digit_ratio("a1b2") - 0.5).abs() < 1e-9);
    }
}

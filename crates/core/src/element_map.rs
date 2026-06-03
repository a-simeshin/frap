//! Element map discovery: cluster DOM snapshot elements for Page Object generation.

use crate::CoreError;
use healing::{DOMElementInfo, DOMSnapshot, HealingEngine};
use serde::{Deserialize, Serialize};
use signature::Signature;
use std::collections::{HashMap, HashSet};

const INTERACTIVE_TAGS: &[&str] = &["button", "a", "input", "select", "textarea"];

/// Test-attribute names recognised as the most stable locator signal, in priority order.
/// `data-testid` is canonical; the rest are common framework aliases.
const TEST_ATTRS: &[&str] = &[
    "data-testid",
    "data-test-id",
    "data-qa",
    "data-cy",
    "data-test",
    "data-automation-id",
];

/// Maximum length of visible text still considered usable as a `text` locator.
const MAX_TEXT_LOCATOR_LEN: usize = 40;

// Base stability score per cascade strategy (before the uniqueness factor).
//
// This ordering encodes the cascade priority `testid > href > role > placeholder > text > id > css`.
// In particular `href` (0.92) outranks `role` (0.90) — a stable route is a better handle than a
// generic role+name — while `role` outranks `text` (0.80) so a unique (role, name) pair wins over a
// bare visible-text match. The positional `css` base (0.40) sits below every *named* strategy so any
// named candidate (even a non-unique one demoted by the uniqueness factor down toward, but never
// below, its own floor) is preferred over the positional fallback.
const BASE_TESTID: f64 = 0.98;
const BASE_HREF: f64 = 0.92;
const BASE_ROLE: f64 = 0.90;
const BASE_PLACEHOLDER: f64 = 0.88;
const BASE_TEXT: f64 = 0.80;
const BASE_ID: f64 = 0.75;
const BASE_CSS: f64 = 0.40;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MapOptions {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub include_non_interactive: bool,
    #[serde(default)]
    pub max_elements: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FilterSpec {
    #[serde(default)]
    pub interactive_only: bool,
    #[serde(default)]
    pub min_cluster_size: Option<usize>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocatorRecommendation {
    pub selector: String,
    pub strategy: String,
    pub confidence: f64,
    /// Normalized strategy value for a future framework-neutral mapping (e.g. the href,
    /// visible text, accessible name, placeholder or attribute value). `None` for the
    /// positional `css` fallback. Skipped when `None` to keep the legacy JSON output stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// SCOPE: the container identity (`scope_hint`) the locator is scoped to, set only when the
    /// element's (role, name) / text is NOT globally unique but IS unique within this scope and the
    /// (scope_hint, role, name) tuple matches exactly one element snapshot-wide (VERIFY). The
    /// generator narrows the query to this container before applying the strategy. `None` for a
    /// plain globally-unique or positional locator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// SCOPE/relative: the per-member discriminator (accessible name / visible text) used to single
    /// out one member of a repeated `List` via a relative locator (the actual relative-selector
    /// generation is `generator-relative`'s job; here we only carry the text and keep `strategy`
    /// meaningful, e.g. `"role"` / `"text"`). `None` for non-list / non-scoped locators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_text: Option<String>,
    /// How many elements the chosen locator matches snapshot-wide. `None` or `Some(1)` means the
    /// locator is unique; `Some(n > 1)` signals the generator to add `.first()` (or index). Only
    /// set when uniqueness data is available (the snapshot-aware cascade).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementNode {
    pub id: String,
    pub selector: String,
    pub recommended_selector: String,
    pub tag: String,
    pub signature: Signature,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    pub confidence: f64,
    pub locator: LocatorRecommendation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClusterType {
    #[default]
    Single,
    List,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Cluster {
    pub id: String,
    pub cluster_type: ClusterType,
    pub element_ids: Vec<String>,
    pub prefix_signature: String,
    /// CLASSIFY enrichment (List clusters only; `None` for `Single`).
    /// Common container role of the repeated component — the role shared by every member's
    /// scope (e.g. `"list"` / `"listitem"` / `"navigation"`). Derived from the members'
    /// `scope_hint` / common `computed_role`; `None` when no shared container role is evident.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_role: Option<String>,
    /// Role of the interactive member itself — its `computed_role` (e.g. `"link"` / `"button"`)
    /// when all members agree, otherwise the effective role inferred from the tag. `None` when
    /// members disagree on role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_role: Option<String>,
    /// What VARIES between the members (the meaningful discriminator): `"text"` when the
    /// accessible name / visible text differs, `"href"` when the href template's masked
    /// (`*`) segment differs. `None` when members are not meaningfully distinguishable
    /// (e.g. only a generated hash differs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub element_count: usize,
    pub cluster_count: usize,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementMap {
    pub elements: Vec<ElementNode>,
    pub clusters: Vec<Cluster>,
    pub metadata: MapMetadata,
}

/// Snapshot-wide value→count indexes, built once per `build_element_map` pass so that
/// uniqueness lookups during cascade evaluation stay O(1) instead of O(n) per element.
#[derive(Debug, Default)]
struct SnapshotIndexes {
    test_attrs: HashMap<(String, String), usize>,
    aria_label: HashMap<String, usize>,
    role: HashMap<String, usize>,
    placeholder: HashMap<String, usize>,
    href: HashMap<String, usize>,
    text: HashMap<String, usize>,
    class: HashMap<String, usize>,
    id: HashMap<String, usize>,
    /// Count per (effective role, accessible name) pair. The role is the explicit `role`
    /// attribute when present, otherwise the implicit ARIA role inferred from the tag; the
    /// accessible name is `aria-label` when present, otherwise the normalized visible text.
    role_name: HashMap<(String, String), usize>,
    /// SCOPE: count per (scope_hint, effective role, accessible name) tuple. Lets the cascade
    /// rescue an element whose (role, name) is ambiguous globally but unique within its container.
    /// VERIFY uses this to confirm a scoped locator matches exactly one element snapshot-wide.
    scoped_role_name: HashMap<(String, String, String), usize>,
    /// SCOPE: count per (scope_hint, normalized text) tuple — the text analogue of
    /// `scoped_role_name` for elements that carry text but no usable role.
    scoped_text: HashMap<(String, String), usize>,
}

impl SnapshotIndexes {
    /// Build all counters in a single pass over `elements` (no O(n^2) recomputation).
    ///
    /// FIX 4 — uniqueness among VISIBLE elements: counters only tally elements whose `visible`
    /// is not `Some(false)` (visible or unknown). A locator that "collides" only with a hidden
    /// element is still effectively unique for the user, so hidden elements must not inflate the
    /// counts that drive the uniqueness decision.
    fn build(elements: &[DOMElementInfo]) -> Self {
        let mut idx = Self::default();
        for element in elements {
            if element.visible == Some(false) {
                continue;
            }
            for attr in TEST_ATTRS {
                if let Some(value) = element.attributes.get(*attr) {
                    *idx.test_attrs
                        .entry(((*attr).to_string(), value.clone()))
                        .or_insert(0) += 1;
                }
            }
            if let Some(value) = element.attributes.get("aria-label") {
                *idx.aria_label.entry(value.clone()).or_insert(0) += 1;
            }
            if let Some(value) = element.attributes.get("role") {
                *idx.role.entry(value.clone()).or_insert(0) += 1;
            }
            if let Some(value) = element.attributes.get("placeholder") {
                *idx.placeholder.entry(value.clone()).or_insert(0) += 1;
            }
            if let Some(value) = element.attributes.get("href") {
                *idx.href.entry(value.clone()).or_insert(0) += 1;
            }
            if let Some(text) = normalized_text(element) {
                *idx.text.entry(text).or_insert(0) += 1;
            }
            for class in classes(element) {
                *idx.class.entry(class).or_insert(0) += 1;
            }
            if let Some(id) = element.attributes.get("id") {
                *idx.id.entry(id.clone()).or_insert(0) += 1;
            }
            if let (Some(role), Some(name)) = (effective_role(element), accessible_name(element)) {
                *idx.role_name
                    .entry((role.clone(), name.clone()))
                    .or_insert(0) += 1;
                if let Some(scope) = scope_of(element) {
                    *idx.scoped_role_name.entry((scope, role, name)).or_insert(0) += 1;
                }
            }
            if let (Some(scope), Some(text)) = (scope_of(element), normalized_text(element)) {
                *idx.scoped_text.entry((scope, text)).or_insert(0) += 1;
            }
        }
        idx
    }
}

/// The element's scope identity used for SCOPE-relative uniqueness: the collector `scope_hint`
/// (identity of the nearest stable container). `None` (and trimmed-empty) means "no usable scope",
/// so the element cannot participate in scoped uniqueness.
fn scope_of(element: &DOMElementInfo) -> Option<String> {
    element
        .scope_hint
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Implicit ARIA role inferred from the tag (and `input[type=…]`), used when no explicit
/// `role` attribute is present. Returns `None` for tags with no well-defined default role.
fn implicit_role(element: &DOMElementInfo) -> Option<&'static str> {
    let tag = element.tag.to_ascii_lowercase();
    match tag.as_str() {
        "a" => Some("link"),
        "button" => Some("button"),
        "select" => Some("combobox"),
        "textarea" => Some("textbox"),
        "img" => Some("img"),
        "nav" => Some("navigation"),
        "input" => {
            let input_type = element
                .attributes
                .get("type")
                .map(|t| t.to_ascii_lowercase())
                .unwrap_or_else(|| "text".to_string());
            match input_type.as_str() {
                "text" | "search" | "email" | "tel" | "url" | "password" => Some("textbox"),
                "checkbox" => Some("checkbox"),
                "radio" => Some("radio"),
                "button" | "submit" | "reset" => Some("button"),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Effective ARIA role: the collector's `computed_role` when present (the authoritative source),
/// else the explicit `role` attribute (it wins over the implicit one), else the implicit role
/// inferred from the tag. `None` when the element has no usable role.
fn effective_role(element: &DOMElementInfo) -> Option<String> {
    if let Some(role) = element
        .computed_role
        .as_ref()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
    {
        return Some(role.to_ascii_lowercase());
    }
    if let Some(role) = element
        .attributes
        .get("role")
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
    {
        return Some(role.to_ascii_lowercase());
    }
    implicit_role(element).map(|r| r.to_string())
}

/// Accessible name: the collector's `accessible_name` when present (the authoritative source),
/// else `aria-label`, else the normalized visible text capped at `MAX_TEXT_LOCATOR_LEN`. `None`
/// when none yields a usable name.
fn accessible_name(element: &DOMElementInfo) -> Option<String> {
    if let Some(name) = element
        .accessible_name
        .as_ref()
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
    {
        return Some(name.to_string());
    }
    if let Some(label) = element
        .attributes
        .get("aria-label")
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
    {
        return Some(label.to_string());
    }
    normalized_text(element).filter(|t| t.chars().count() <= MAX_TEXT_LOCATOR_LEN)
}

/// A single cascade candidate: its locator plus the strategy base score.
struct Candidate {
    selector: String,
    strategy: &'static str,
    base: f64,
    count: usize,
    /// Normalized strategy value (href / text / accessible name / placeholder / attribute
    /// value / id / class). `None` for the positional `css` fallback.
    value: Option<String>,
    /// SCOPE: the container (`scope_hint`) this candidate is scoped to. `Some` only for
    /// scoped-uniqueness candidates that VERIFY confirmed unique within that scope; `None` for a
    /// plain global candidate. Propagated to `LocatorRecommendation::scope`.
    scope: Option<String>,
    /// SCOPE/relative: per-member discriminator for a `List` member, propagated to
    /// `LocatorRecommendation::filter_text`. `None` for non-list candidates.
    filter_text: Option<String>,
    /// `true` when this candidate is the positional `css` fallback — used to exclude it from the
    /// "max-base among unique" pass so any *named* candidate is preferred (FIX 1).
    positional: bool,
}

impl Candidate {
    /// A plain global (non-scoped, non-positional) named candidate.
    fn named(
        selector: String,
        strategy: &'static str,
        base: f64,
        count: usize,
        value: Option<String>,
    ) -> Self {
        Candidate {
            selector,
            strategy,
            base,
            count,
            value,
            scope: None,
            filter_text: None,
            positional: false,
        }
    }
}

/// Thin public wrapper: single-element best-effort cascade with **no** uniqueness data.
/// Every candidate is treated as unique, so `confidence` equals the strategy base score.
/// This is a deliberate degradation of the single-element API (locked in by a test).
pub fn recommend_locator(element: &DOMElementInfo) -> LocatorRecommendation {
    let candidates = build_candidates(element, None);
    finalize(candidates, element)
}

/// Snapshot-aware cascade: picks the first candidate whose selector is unique across the
/// whole snapshot, applying a uniqueness factor to its confidence.
fn recommend_locator_in_snapshot(
    element: &DOMElementInfo,
    indexes: &SnapshotIndexes,
) -> LocatorRecommendation {
    let candidates = build_candidates(element, Some(indexes));
    finalize(candidates, element)
}

/// Resolve the cascade to a single recommendation.
///
/// Priority (LOCATE combination 3 — max-base among unique, fixes the priority-inversion the old
/// "first unique in order" logic had):
///   1. Among the *named* (non-positional) candidates that are globally unique (`count <= 1`),
///      pick the one with the **highest base** (so `href` 0.92 beats `role` 0.90 beats `text`
///      0.80, regardless of insertion order). This is the strongest invariant: a unique named
///      candidate always wins, and the best base among them is chosen.
///   2. Otherwise fall back to the highest-base named candidate even if non-unique, with the
///      uniqueness penalty applied to its confidence (a non-unique named locator still beats the
///      positional one, since every named base sits above `BASE_CSS`).
///   3. Only if there is no named candidate at all do we emit the positional `css` fallback.
///
/// Invariant: unique named (max base) > non-unique named > positional.
fn finalize(candidates: Vec<Candidate>, element: &DOMElementInfo) -> LocatorRecommendation {
    // 1. Highest-base candidate among globally-unique NAMED candidates (positional excluded).
    let unique_best = candidates
        .iter()
        .filter(|c| !c.positional && c.count <= 1)
        .max_by(|a, b| {
            a.base
                .partial_cmp(&b.base)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some(best) = unique_best {
        return recommendation_from(best);
    }

    // 2. No unique named candidate: highest-base NAMED candidate (non-unique), penalised.
    let named_best = candidates.iter().filter(|c| !c.positional).max_by(|a, b| {
        a.base
            .partial_cmp(&b.base)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(best) = named_best {
        return recommendation_from(best);
    }

    // 3. No named signal at all: the positional candidate if one exists, else an anchored fallback.
    if let Some(positional) = candidates.iter().find(|c| c.positional) {
        return recommendation_from(positional);
    }
    LocatorRecommendation {
        selector: anchor_positional_selector(&element.selector),
        strategy: "css".to_string(),
        confidence: clamp01(BASE_CSS),
        value: None,
        scope: None,
        filter_text: None,
        match_count: None,
    }
}

/// Build a `LocatorRecommendation` from a chosen candidate, applying the uniqueness penalty to its
/// confidence and reporting `match_count` (`None` when unique-by-construction, `Some(n)` otherwise).
fn recommendation_from(c: &Candidate) -> LocatorRecommendation {
    let match_count = if c.count > 1 {
        Some(c.count as u32)
    } else {
        Some(1)
    };
    LocatorRecommendation {
        selector: c.selector.clone(),
        strategy: c.strategy.to_string(),
        confidence: clamp01(c.base * uniqueness_factor(c.count)),
        value: c.value.clone(),
        scope: c.scope.clone(),
        filter_text: c.filter_text.clone(),
        match_count,
    }
}

/// Build the ordered cascade of candidates for one element.
/// `indexes` provides snapshot-wide counts; when `None`, every candidate counts as unique.
fn build_candidates(element: &DOMElementInfo, indexes: Option<&SnapshotIndexes>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    // 1. test attributes (data-testid + aliases)
    for attr in TEST_ATTRS {
        if let Some(value) = element.attributes.get(*attr) {
            let count = indexes
                .map(|i| {
                    i.test_attrs
                        .get(&((*attr).to_string(), value.clone()))
                        .copied()
                        .unwrap_or(1)
                })
                .unwrap_or(1);
            out.push(Candidate::named(
                format!("[{attr}=\"{value}\"]"),
                "testid",
                BASE_TESTID,
                count,
                Some(value.clone()),
            ));
        }
    }

    // 2. role + accessible name
    if let Some(label) = element.attributes.get("aria-label") {
        let count = indexes
            .map(|i| i.aria_label.get(label).copied().unwrap_or(1))
            .unwrap_or(1);
        out.push(Candidate::named(
            format!("[aria-label=\"{label}\"]"),
            "role",
            BASE_ROLE,
            count,
            Some(label.clone()),
        ));
    } else if let Some(role) = element.attributes.get("role") {
        let count = indexes
            .map(|i| i.role.get(role).copied().unwrap_or(1))
            .unwrap_or(1);
        out.push(Candidate::named(
            format!("[role=\"{role}\"]"),
            "role",
            BASE_ROLE,
            count,
            Some(role.clone()),
        ));
    }

    // 3. placeholder
    if let Some(placeholder) = element.attributes.get("placeholder") {
        let count = indexes
            .map(|i| i.placeholder.get(placeholder).copied().unwrap_or(1))
            .unwrap_or(1);
        out.push(Candidate::named(
            format!("[placeholder=\"{placeholder}\"]"),
            "placeholder",
            BASE_PLACEHOLDER,
            count,
            Some(placeholder.clone()),
        ));
    }

    // 4. href (anchors only). A `#` / empty href is a no-op anchor (placeholder / JS hook), not a
    //    real route, so it is NOT a usable locator value — treat it as "no href" and skip.
    if element.tag.eq_ignore_ascii_case("a") {
        if let Some(href) = element.attributes.get("href").filter(|h| is_real_href(h)) {
            let count = indexes
                .map(|i| i.href.get(href).copied().unwrap_or(1))
                .unwrap_or(1);
            out.push(Candidate::named(
                format!("[href=\"{href}\"]"),
                "href",
                BASE_HREF,
                count,
                Some(href.clone()),
            ));
        }
    }

    // 5. short unique visible text. Playwright CSS cannot express text directly, so the
    //    selector stays positional; the `text` strategy/confidence carry the semantic intent
    //    (a future task maps this through a `value` field).
    if let Some(text) = normalized_text(element) {
        if text.chars().count() <= MAX_TEXT_LOCATOR_LEN {
            let count = indexes
                .map(|i| i.text.get(&text).copied().unwrap_or(1))
                .unwrap_or(1);
            out.push(Candidate::named(
                element.selector.clone(),
                "text",
                BASE_TEXT,
                count,
                Some(text.clone()),
            ));
        }
    }

    // 5b. role + accessible name (implicit/explicit role × aria-label/visible text).
    //     This "rescues" elements without stable attributes whose bare text is NOT globally
    //     unique (so the `text` tier above does not fire on uniqueness), but whose
    //     (role, name) pair IS unique on the page — lifting them out of positional CSS into a
    //     semantic `role` locator. `value` carries the accessible name; the selector stays
    //     positional (Playwright role queries are not expressible as a CSS selector). The
    //     generator distinguishes getByRole vs getByLabel by the presence of `aria-label`.
    if let (Some(role), Some(name)) = (effective_role(element), accessible_name(element)) {
        let count = indexes
            .map(|i| i.role_name.get(&(role.clone(), name.clone())).copied())
            .unwrap_or(Some(1))
            .unwrap_or(1);
        out.push(Candidate::named(
            element.selector.clone(),
            "role",
            BASE_ROLE,
            count,
            Some(name.clone()),
        ));

        // SCOPE (combination 1) + VERIFY: when the global (role, name) is NOT unique but the
        // (scope_hint, role, name) tuple matches exactly ONE element snapshot-wide, emit a
        // scoped candidate. VERIFY = "matches exactly 1 in the scoped index"; if the tuple is
        // not globally unambiguous we do NOT emit it and let the cascade degrade. The scoped
        // candidate reuses the role base (it is still a role+name locator, just narrowed to a
        // container) and `count = 1` so it is treated as unique.
        if count > 1 {
            if let (Some(idx), Some(scope)) = (indexes, scope_of(element)) {
                let scoped_count = idx
                    .scoped_role_name
                    .get(&(scope.clone(), role.clone(), name.clone()))
                    .copied()
                    .unwrap_or(0);
                if scoped_count == 1 {
                    out.push(Candidate {
                        selector: element.selector.clone(),
                        strategy: "role",
                        base: BASE_ROLE,
                        count: 1,
                        value: Some(name.clone()),
                        scope: Some(scope),
                        // The name is also the per-member discriminator for a relative locator.
                        filter_text: Some(name),
                        positional: false,
                    });
                }
            }
        }
    } else if let (Some(idx), Some(scope), Some(text)) =
        (indexes, scope_of(element), normalized_text(element))
    {
        // SCOPE for text-only elements (no usable role): a globally non-unique text that is
        // unique within its scope, VERIFY-confirmed via the scoped_text index (== 1).
        if text.chars().count() <= MAX_TEXT_LOCATOR_LEN {
            let global = idx.text.get(&text).copied().unwrap_or(1);
            let scoped = idx
                .scoped_text
                .get(&(scope.clone(), text.clone()))
                .copied()
                .unwrap_or(0);
            if global > 1 && scoped == 1 {
                out.push(Candidate {
                    selector: element.selector.clone(),
                    strategy: "text",
                    base: BASE_TEXT,
                    count: 1,
                    value: Some(text.clone()),
                    scope: Some(scope),
                    filter_text: Some(text),
                    positional: false,
                });
            }
        }
    }

    // 6. non-generated id, then non-generated unique class.
    if let Some(id) = element.attributes.get("id") {
        if !signature::looks_like_generated(id) {
            let count = indexes
                .map(|i| i.id.get(id).copied().unwrap_or(1))
                .unwrap_or(1);
            out.push(Candidate::named(
                format!("#{}", escape_css_attr(id)),
                "id",
                BASE_ID,
                count,
                Some(id.clone()),
            ));
        }
    }
    for class in classes(element) {
        if signature::looks_like_generated(&class) {
            continue;
        }
        let count = indexes
            .map(|i| i.class.get(&class).copied().unwrap_or(1))
            .unwrap_or(1);
        out.push(Candidate::named(
            format!(".{}", escape_css_attr(&class)),
            "css",
            BASE_ID,
            count,
            Some(class.clone()),
        ));
    }

    // 7. positional CSS fallback (always unique-by-construction anchor): no normalized value.
    //     Anchored to the nearest stable token in the element's own selector instead of the
    //     full long nth-of-type chain (see `anchor_positional_selector`). Marked `positional` so
    //     the cascade only reaches for it when no named candidate exists (FIX 1).
    out.push(Candidate {
        selector: anchor_positional_selector(&element.selector),
        strategy: "css",
        base: BASE_CSS,
        count: 1,
        value: None,
        scope: None,
        filter_text: None,
        positional: true,
    });

    out
}

/// `true` when an `href` value is a real navigable route rather than a no-op placeholder.
/// `#`, an empty / whitespace-only value, and a bare `javascript:` hook are NOT real hrefs and must
/// not produce an `href` locator candidate.
fn is_real_href(href: &str) -> bool {
    let trimmed = href.trim();
    !(trimmed.is_empty() || trimmed == "#")
}

/// Shorten a positional CSS selector by anchoring it on the nearest stable token in its own
/// descendant chain. When a compound segment carries a stable token (an `#id`, a
/// `[data-testid…]`/`[data-test…]` attribute, or a non-generated class — `looks_like_generated`
/// rejects runtime hashes), the chain is trimmed to start at that segment and keep the tail
/// after it. Falls back to the original selector when no stable anchor exists.
///
/// `div > ul > li.card > a:nth-of-type(2)` -> `li.card > a:nth-of-type(2)`.
fn anchor_positional_selector(selector: &str) -> String {
    let trimmed = selector.trim();
    if trimmed.is_empty() {
        return selector.to_string();
    }
    // Split on descendant/child combinators while remembering the original separators so the
    // rebuilt selector keeps its shape.
    let segments: Vec<&str> = trimmed.split(" > ").collect();
    if segments.len() <= 1 {
        return selector.to_string();
    }
    // Find the LAST segment (closest to the target) that carries a stable token; anchoring as
    // deep as possible yields the shortest still-stable selector.
    let anchor = segments
        .iter()
        .rposition(|seg| segment_has_stable_token(seg));
    match anchor {
        Some(idx) if idx > 0 => segments[idx..].join(" > "),
        _ => selector.to_string(),
    }
}

/// `true` when a single compound CSS segment (e.g. `li.card#x[data-testid="y"]:nth-of-type(2)`)
/// contains at least one token that is stable enough to anchor a selector on.
fn segment_has_stable_token(segment: &str) -> bool {
    // `#id` token: stable unless it reads as a generated hash.
    if let Some(rest) = segment.split('#').nth(1) {
        let id: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !id.is_empty() && !signature::looks_like_generated(&id) {
            return true;
        }
    }
    // `[data-testid…]` / `[data-test…]` attribute token.
    if segment.contains("[data-testid") || segment.contains("[data-test") {
        return true;
    }
    // Non-generated class token(s).
    for class in segment.split('.').skip(1) {
        let name: String = class
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !name.is_empty() && !signature::looks_like_generated(&name) {
            return true;
        }
    }
    false
}

/// Uniqueness multiplier: 1 match = 1.0, 2 matches = 0.6, 3+ matches = 0.4.
fn uniqueness_factor(count: usize) -> f64 {
    match count {
        0 | 1 => 1.0,
        2 => 0.6,
        _ => 0.4,
    }
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// Trimmed visible text, `None` when empty.
fn normalized_text(element: &DOMElementInfo) -> Option<String> {
    element
        .text_content
        .as_ref()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Class tokens from the `class` attribute (whitespace-separated, non-empty).
fn classes(element: &DOMElementInfo) -> Vec<String> {
    element
        .attributes
        .get("class")
        .map(|c| c.split_whitespace().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// Semantic clustering key. Two elements share a `List` cluster only when this key is
/// identical, i.e. they are genuinely homogeneous (same tag-or-role AND the same accessible
/// shape / href pattern). This deliberately replaces the old `signature.prefix` key, which
/// collapsed hundreds of unrelated div-soup nodes into one mega-"List".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SemanticKey {
    /// `role` attribute when present, otherwise the lowercased tag — the primary kind.
    tag_or_role: String,
    /// Whether the element carries an `href` (anchors / link-like patterns).
    has_href: bool,
    /// Normalized full href path *template*: the whole path (not just the first segment),
    /// with variable-looking segments (numeric ids, uuids, runtime hashes) masked to `*`.
    /// `/products/12` and `/products/13` -> `products/*` (a templated repeated list), while
    /// `/ru/person` and `/ru/business` stay distinct (`ru/person` vs `ru/business`). This
    /// replaces the old first-segment-only prefix that collapsed every link under a shared
    /// top-level section (e.g. all `/ru/...`) into one mega-`List`. `None` when no usable path.
    href_path_template: Option<String>,
    /// Coarse shape of the accessible name: each maximal run of letters becomes `L`,
    /// digits `D`, everything else a single space. `"Card 1"`/`"Card 2"` -> `"L D"`,
    /// so repeated cards collapse while structurally different text stays distinct.
    /// `None` when the element has no usable visible text.
    text_shape: Option<String>,
}

/// Build the semantic key for one element (see `SemanticKey`).
fn semantic_key(element: &DOMElementInfo) -> SemanticKey {
    let tag_or_role = element
        .attributes
        .get("role")
        .filter(|r| !r.trim().is_empty())
        .map(|r| r.trim().to_lowercase())
        .unwrap_or_else(|| element.tag.to_lowercase());

    let href = element.attributes.get("href");
    let has_href = href.is_some();
    let href_path_template = href.and_then(|h| href_path_template(h));

    let text_shape = normalized_text(element).map(|t| text_shape(&t));

    SemanticKey {
        tag_or_role,
        has_href,
        href_path_template,
        text_shape,
    }
}

/// Whether a single href path segment looks like a variable identifier rather than a stable
/// route name: purely numeric, a UUID, or a runtime-generated hash. These are masked to `*`
/// in the href template so that `/products/1`, `/products/2`, … collapse to one templated
/// pattern (a real repeated list) while textual routes (`/person`, `/business`) stay distinct.
fn segment_is_variable(segment: &str) -> bool {
    if segment.is_empty() {
        return false;
    }
    if segment.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    // Mixed alphanumeric tokens that read as generated hashes / uuids (e.g. `a1b2c3d4`,
    // `f47ac10b-58cc-…`). Reuses the signature heuristic so we mask exactly what the
    // locator layer already treats as unstable.
    signature::looks_like_generated(segment)
}

/// Normalized full href path *template*, ignoring scheme/host and query/fragment, with
/// variable-looking segments masked to `*` (see `segment_is_variable`).
/// `https://host/products/12?x=1` -> `Some("products/*")`; `/ru/person` -> `Some("ru/person")`;
/// `#` / `/` / empty -> `None`.
fn href_path_template(href: &str) -> Option<String> {
    let trimmed = href.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Drop scheme + host (anything up to the first single slash path component).
    let after_host = trimmed
        .split_once("://")
        .map(|(_, rest)| rest.split_once('/').map(|(_, p)| p).unwrap_or(""))
        .unwrap_or(trimmed);
    // Keep only the path part; strip query/fragment.
    let path = after_host
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches('/');
    let template: Vec<String> = path
        .split('/')
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            if segment_is_variable(seg) {
                "*".to_string()
            } else {
                seg.to_string()
            }
        })
        .collect();
    if template.is_empty() {
        None
    } else {
        Some(template.join("/"))
    }
}

/// Coarse character-class shape used to compare accessible-name patterns.
fn text_shape(text: &str) -> String {
    let mut shape = String::new();
    let mut last: Option<char> = None;
    for ch in text.chars() {
        let class = if ch.is_alphabetic() {
            'L'
        } else if ch.is_numeric() {
            'D'
        } else {
            ' '
        };
        // Collapse consecutive runs of the same class into a single marker.
        if last != Some(class) {
            shape.push(class);
            last = Some(class);
        }
    }
    shape.trim().to_string()
}

/// Deterministic cluster id from the semantic key, stable across runs.
fn cluster_id_for(key: &SemanticKey) -> String {
    let href_part = match (key.has_href, &key.href_path_template) {
        (true, Some(p)) => format!("href_{}", sanitize_id(p)),
        (true, None) => "href".to_string(),
        (false, _) => "nohref".to_string(),
    };
    let text_part = key
        .text_shape
        .as_ref()
        .map(|s| s.replace(' ', "_"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "notext".to_string());
    format!(
        "cluster_{}__{}__{}",
        sanitize_id(&key.tag_or_role),
        href_part,
        sanitize_id(&text_part)
    )
}

/// Decide whether a same-key group of ≥2 members is a genuinely *repeated component* (which
/// should become a `List`, addressed positionally / by a shared pattern) rather than a bag of
/// individually-addressable elements that merely happen to share a tag and a coarse text shape.
///
/// Rule for link-like groups (the mega-`List` offender on real pages): a set of `<a>` whose
/// hrefs are *all distinct* is NOT a repeated component unless they collapse to a real path
/// *template* — i.e. the shared href template actually masked a variable segment (`*`), as in
/// `/products/1`, `/products/2`, `/products/3` -> `products/*`. A group of distinct routes
/// with no masked segment (each a unique destination, e.g. `/person`, `/business`, `/about`)
/// stays `Single`: those are individual semantic locators, not a list to index with `.nth()`.
///
/// Non-link groups (no `has_href`) are unaffected — repeated cards/buttons keyed by tag+text
/// shape (e.g. two `product-card` articles, two "Buy" buttons) remain `List` as before.
fn is_repeated_component(
    key: &SemanticKey,
    element_ids: &[String],
    element_href: &HashMap<String, Option<String>>,
) -> bool {
    if !key.has_href {
        // Repeated structural component keyed by tag/role + text shape (no href signal).
        return true;
    }

    // A templated href path (a masked `*` segment) is the hallmark of a real repeated list:
    // the members share a route pattern and differ only by a variable id. Keep it a `List`.
    if key
        .href_path_template
        .as_deref()
        .is_some_and(|t| t.contains('*'))
    {
        return true;
    }

    // No templated segment: this is a List only if the members are actual repeats, i.e. they
    // do NOT all carry distinct hrefs. All-distinct hrefs => distinct destinations => Single.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut total = 0usize;
    for id in element_ids {
        if let Some(Some(href)) = element_href.get(id) {
            total += 1;
            seen.insert(href.as_str());
        }
    }
    // If every member contributed a distinct href, it is a bag of unique links -> not a list.
    !(total >= 2 && seen.len() == total)
}

fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

// ---------------------------------------------------------------------------
// CLASSIFY stage: confirm SemanticKey clusters with structural similarity and
// derive container_role / child_role / variable_kind for List clusters.
// ---------------------------------------------------------------------------

/// Minimum average pairwise structural similarity (`signature::calculate_confidence`) a
/// SemanticKey group must reach before it is confirmed a *hard repeated component* (a `List`).
/// Groups that key together but are structurally heterogeneous (similarity below this) are
/// dissolved into `Single`s — the SemanticKey alone is not trusted without structural backing.
const LIST_SIMILARITY_THRESHOLD: f64 = 0.7;

/// Per-element signals CLASSIFY needs after the build pass to confirm clusters and derive the
/// container/child roles and the variable discriminator. Indexed by element id.
#[derive(Debug, Clone)]
struct ClassifyInfo {
    /// Structural signature (path/tokens/children) used for pairwise similarity.
    signature: Signature,
    /// Computed ARIA role from the collector, falling back to the role inferred from the tag.
    role: Option<String>,
    /// `scope_hint` from the collector: identity of the nearest stable container.
    scope_hint: Option<String>,
    /// Accessible name / visible text — the "text" discriminator candidate.
    name: Option<String>,
    /// Raw href — the "href" discriminator candidate. Members of a templated list share the same
    /// masked template (`products/*`) but differ in the raw value, so the raw href (not the
    /// masked template) is what reveals the per-member `*`-segment variation.
    href: Option<String>,
}

/// Average pairwise structural similarity across a group's members, computed against the first
/// member (co-anchored) to stay O(n) rather than O(n^2). A single-member group scores `1.0`.
fn average_member_similarity(member_signatures: &[&Signature]) -> f64 {
    match member_signatures.split_first() {
        None | Some((_, [])) => 1.0,
        Some((anchor, rest)) => {
            let sum: f64 = rest
                .iter()
                .map(|sig| signature::calculate_confidence(anchor, sig))
                .sum();
            sum / rest.len() as f64
        }
    }
}

/// Effective role of one classified member: its collector `computed_role`, else `None`.
/// Used to test role homogeneity (every member must agree, not just share a tag).
fn member_role(info: &ClassifyInfo) -> Option<&str> {
    info.role.as_deref()
}

/// `true` when every member of the group agrees on a role (homogeneous by `computed_role`).
/// Members without a role are treated as compatible (absence does not break homogeneity), but
/// two *different* present roles do break it.
fn roles_are_homogeneous(infos: &[&ClassifyInfo]) -> bool {
    let mut seen: Option<&str> = None;
    for info in infos {
        if let Some(role) = member_role(info) {
            match seen {
                Some(prev) if prev != role => return false,
                _ => seen = Some(role),
            }
        }
    }
    true
}

/// Confirm a SemanticKey group as a hard repeated component (`List`): members must be
/// homogeneous by role (`computed_role`) AND structurally similar (average pairwise
/// `calculate_confidence` ≥ `LIST_SIMILARITY_THRESHOLD`). Otherwise the group is dissolved
/// into `Single`s — keying together is necessary but not sufficient.
fn structural_similarity_confirms_list(infos: &[&ClassifyInfo]) -> bool {
    if infos.len() < 2 {
        return false;
    }
    if !roles_are_homogeneous(infos) {
        return false;
    }
    let signatures: Vec<&Signature> = infos.iter().map(|i| &i.signature).collect();
    average_member_similarity(&signatures) >= LIST_SIMILARITY_THRESHOLD
}

/// `true` when the values are all equal (a *constant* signal → a container trait), `false` when
/// at least two differ (a *variable* signal → an element discriminator). An all-`None` set is
/// treated as constant (nothing distinguishes the members on this axis).
fn values_are_constant(values: &[Option<&str>]) -> bool {
    let mut seen: Option<&str> = None;
    for value in values.iter().flatten() {
        match seen {
            Some(prev) if prev != *value => return false,
            _ => seen = Some(*value),
        }
    }
    true
}

/// Whether a discriminator candidate is *meaningful* — at least two distinct values, none of
/// which read as machine-generated hashes (entropy detector). A set whose only differences are
/// generated tokens carries no semantic discriminator.
fn discriminator_is_meaningful(values: &[Option<&str>]) -> bool {
    let mut distinct: HashSet<&str> = HashSet::new();
    for value in values.iter().flatten() {
        if signature::looks_like_generated(value) {
            continue;
        }
        distinct.insert(*value);
    }
    distinct.len() >= 2
}

/// Maps a `scope_hint` (the nearest stable container's identity from the collector) to a
/// container role, but only when the hint is itself a *container* role. A landmark hint
/// (`navigation`, `banner`, …) or a list-structure hint (`listitem`, `list`, `row`, `cell`,
/// `option`, `article`, `region`, `gridcell`, `rowgroup`) is accepted verbatim. Any other hint
/// (e.g. a member's own role leaked in) is rejected so it can't masquerade as the container.
fn container_role_from_scope_hint(hint: &str) -> Option<String> {
    matches!(
        hint,
        "listitem"
            | "list"
            | "row"
            | "cell"
            | "gridcell"
            | "rowgroup"
            | "option"
            | "article"
            | "region"
            | "navigation"
            | "banner"
            | "contentinfo"
            | "main"
            | "complementary"
            | "form"
            | "search"
    )
    .then(|| hint.to_string())
}

/// Maps a *container* HTML tag (the element directly wrapping a repeated member) to the ARIA
/// role of the repeated UNIT. The relative-locator pattern keys off the repeating unit's role
/// (e.g. `listitem`), not the surrounding collection (`list`), so a `<li>` parent yields
/// `"listitem"`. Returns `None` for non-informative wrappers (`div`/`span`) so the caller can
/// climb to the nearest meaningful ancestor.
fn container_role_from_tag(tag: &str) -> Option<&'static str> {
    match tag {
        "li" => Some("listitem"),
        "tr" => Some("row"),
        "td" | "th" => Some("cell"),
        "option" => Some("option"),
        "article" => Some("article"),
        "section" => Some("region"),
        // A bare `<ul>`/`<ol>` is the collection, not the unit; the relative pattern wants the
        // repeated unit's role, so a list collection maps to the listitem it contains.
        "ul" | "ol" => Some("listitem"),
        _ => None,
    }
}

/// Tag of a `"tag:role"` path segment (the part before the first `:`).
fn segment_tag(token: &signature::DOMToken) -> &str {
    token.tag.as_str()
}

/// Derives the repeated-unit container role for ONE member from its DOM `path`
/// (`signature.path`, root → element). Starts at the member's parent (the segment directly
/// above it) and climbs toward the root, skipping non-informative wrappers (`div`/`span`),
/// returning the role of the first meaningful container tag. `None` when no informative
/// ancestor exists.
fn container_role_from_path(info: &ClassifyInfo) -> Option<String> {
    let path = &info.signature.path;
    // Last segment is the member itself; its parent is the second-to-last. Walk parents
    // upward (nearest first), past `div`/`span` wrappers, to the first meaningful container.
    let member_index = path.len().checked_sub(1)?;
    path[..member_index]
        .iter()
        .rev()
        .find_map(|token| container_role_from_tag(segment_tag(token)))
        .map(|role| role.to_string())
}

/// Common container role for a confirmed List cluster — the ARIA role of the repeating UNIT,
/// used to anchor the relative locator. Source priority:
/// 1. A `scope_hint` shared by every member that maps to a container role (covers landmark
///    ancestors and list structures the collector already labelled);
/// 2. otherwise the role inferred from the members' DOM `path` — the nearest meaningful
///    container tag above each member (`li` → `listitem`, `tr` → `row`, …), so a bare
///    `<ul><li><a>` list (no landmark, no scope_hint) still anchors a relative locator.
///
/// Returns `None` only when members disagree or neither source yields a container role, in
/// which case the generator falls back to positional `.nth(index)`.
fn derive_container_role(infos: &[&ClassifyInfo]) -> Option<String> {
    let hints: Vec<Option<&str>> = infos.iter().map(|i| i.scope_hint.as_deref()).collect();
    if values_are_constant(&hints) {
        if let Some(role) = hints
            .into_iter()
            .flatten()
            .next()
            .and_then(container_role_from_scope_hint)
        {
            return Some(role);
        }
    }

    // Fall back to the path-derived container role, but only when every member agrees on it —
    // a heterogeneous cluster has no single repeating unit to anchor against.
    let roles: Vec<Option<String>> = infos.iter().map(|i| container_role_from_path(i)).collect();
    let role_refs: Vec<Option<&str>> = roles.iter().map(|r| r.as_deref()).collect();
    if !values_are_constant(&role_refs) {
        return None;
    }
    roles.into_iter().flatten().next()
}

/// Role of the interactive member itself — its `computed_role` when all members agree.
/// `None` when members disagree on role.
fn derive_child_role(infos: &[&ClassifyInfo]) -> Option<String> {
    if !roles_are_homogeneous(infos) {
        return None;
    }
    infos
        .iter()
        .find_map(|i| member_role(i))
        .map(|s| s.to_string())
}

/// What meaningfully VARIES across a confirmed List cluster's members.
/// - `"text"` when the accessible name / visible text differs (and is not a hash);
/// - `"href"` when the masked href template's `*`-segment differs (and is not a hash);
/// - `None` when neither axis carries a meaningful discriminator (members are interchangeable).
///
/// Text wins over href when both vary, as the visible name is the more user-facing handle.
fn derive_variable_kind(infos: &[&ClassifyInfo]) -> Option<String> {
    let names: Vec<Option<&str>> = infos.iter().map(|i| i.name.as_deref()).collect();
    if !values_are_constant(&names) && discriminator_is_meaningful(&names) {
        return Some("text".to_string());
    }
    let hrefs: Vec<Option<&str>> = infos.iter().map(|i| i.href.as_deref()).collect();
    if !values_are_constant(&hrefs) && discriminator_is_meaningful(&hrefs) {
        return Some("href".to_string());
    }
    None
}

pub fn build_element_map(snapshot: &DOMSnapshot, options: &MapOptions) -> ElementMap {
    let mut cluster_members: HashMap<String, Vec<String>> = HashMap::new();
    let mut cluster_prefix: HashMap<String, String> = HashMap::new();
    let mut cluster_keys: HashMap<String, SemanticKey> = HashMap::new();
    let mut cluster_order: Vec<String> = Vec::new();
    let mut elements = Vec::new();
    // Per-element href, indexed by element id, so the cluster-finalization pass can tell a
    // templated repeated list (shared template, e.g. `products/*`) from a bag of distinct
    // routes that merely share a tag/text shape.
    let mut element_href: HashMap<String, Option<String>> = HashMap::new();
    // Per-element CLASSIFY signals (signature for similarity, role/scope/name/href for the
    // container/child-role and variable-kind derivation), indexed by element id.
    let mut classify_info: HashMap<String, ClassifyInfo> = HashMap::new();

    let limit = options.max_elements.unwrap_or(snapshot.elements.len());
    let indexes = SnapshotIndexes::build(&snapshot.elements);

    for (idx, element) in snapshot.elements.iter().take(limit).enumerate() {
        if !options.include_non_interactive && !is_interactive(element) {
            continue;
        }

        let signature = HealingEngine::signature_from_element(element);
        // Semantic clustering: group only genuinely homogeneous elements (same tag/role +
        // same href/text shape), not everything that happens to share a tag prefix.
        let key = semantic_key(element);
        let cluster_id = cluster_id_for(&key);
        let element_id = format!("el-{idx}");
        cluster_prefix
            .entry(cluster_id.clone())
            .or_insert_with(|| signature.prefix.clone());
        cluster_keys
            .entry(cluster_id.clone())
            .or_insert_with(|| key.clone());
        cluster_members
            .entry(cluster_id.clone())
            .or_insert_with(|| {
                cluster_order.push(cluster_id.clone());
                Vec::new()
            })
            .push(element_id.clone());
        element_href.insert(element_id.clone(), element.attributes.get("href").cloned());
        classify_info.insert(
            element_id.clone(),
            ClassifyInfo {
                signature: signature.clone(),
                // Prefer the collector's computed role; fall back to the role inferred from the
                // tag/attributes so role homogeneity still works on legacy snapshots.
                role: element
                    .computed_role
                    .clone()
                    .or_else(|| effective_role(element)),
                scope_hint: element.scope_hint.clone(),
                // Prefer the collector's accessible name; fall back to visible text.
                name: element
                    .accessible_name
                    .clone()
                    .or_else(|| accessible_name(element)),
                href: element.attributes.get("href").cloned(),
            },
        );

        let locator = recommend_locator_in_snapshot(element, &indexes);
        // Node confidence mirrors the chosen locator so downstream conf>=0.8 metrics align.
        let confidence = locator.confidence;

        elements.push(ElementNode {
            id: element_id,
            selector: element.selector.clone(),
            recommended_selector: locator.selector.clone(),
            tag: element.tag.clone(),
            signature,
            cluster_id: Some(cluster_id),
            confidence,
            locator,
        });
    }

    // A cluster is a `List` only when ≥2 homogeneous members share the semantic key, are a
    // genuinely repeated component (`is_repeated_component`), AND the SemanticKey grouping is
    // confirmed by structural similarity + role homogeneity (`structural_similarity_confirms_list`).
    // A group that keys together but is structurally heterogeneous is dissolved into `Single`s.
    // Confirmed List clusters are enriched with container_role / child_role / variable_kind.
    let clusters = cluster_order
        .into_iter()
        .map(|id| {
            let element_ids = cluster_members.remove(&id).unwrap_or_default();
            let key = cluster_keys.get(&id);
            let infos: Vec<&ClassifyInfo> = element_ids
                .iter()
                .filter_map(|eid| classify_info.get(eid))
                .collect();

            let keyed_repeat = element_ids.len() >= 2
                && key.is_some_and(|k| is_repeated_component(k, &element_ids, &element_href));
            // SemanticKey + structural similarity must BOTH agree for a hard List component.
            let is_list = keyed_repeat && structural_similarity_confirms_list(&infos);

            let mut cluster = Cluster {
                id: id.clone(),
                cluster_type: if is_list {
                    ClusterType::List
                } else {
                    ClusterType::Single
                },
                element_ids,
                prefix_signature: cluster_prefix.get(&id).cloned().unwrap_or_default(),
                container_role: None,
                child_role: None,
                variable_kind: None,
            };
            if is_list {
                cluster.container_role = derive_container_role(&infos);
                cluster.child_role = derive_child_role(&infos);
                cluster.variable_kind = derive_variable_kind(&infos);
            }
            cluster
        })
        .collect::<Vec<_>>();

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    ElementMap {
        elements,
        clusters,
        metadata: MapMetadata {
            url: options.url.clone(),
            element_count: 0,
            cluster_count: 0,
            timestamp_ms,
        },
    }
    .with_metadata_counts()
}

impl ElementMap {
    fn with_metadata_counts(mut self) -> Self {
        self.metadata.element_count = self.elements.len();
        self.metadata.cluster_count = self.clusters.len();
        self
    }
}

pub fn filter_element_map(map: &ElementMap, spec: &FilterSpec) -> ElementMap {
    let allowed_tags: Option<HashSet<&str>> = spec
        .tags
        .as_ref()
        .map(|t| t.iter().map(|s| s.as_str()).collect());

    let mut kept_ids: HashSet<String> = HashSet::new();

    for element in &map.elements {
        if spec.interactive_only && !is_interactive_tag(&element.tag) {
            continue;
        }
        if let Some(tags) = &allowed_tags {
            if !tags.contains(element.tag.as_str()) {
                continue;
            }
        }
        if let Some(min_size) = spec.min_cluster_size {
            if let Some(cluster_id) = &element.cluster_id {
                let size = map
                    .clusters
                    .iter()
                    .find(|c| &c.id == cluster_id)
                    .map(|c| c.element_ids.len())
                    .unwrap_or(0);
                if size < min_size {
                    continue;
                }
            } else if min_size > 1 {
                continue;
            }
        }
        kept_ids.insert(element.id.clone());
    }

    let elements: Vec<ElementNode> = map
        .elements
        .iter()
        .filter(|e| kept_ids.contains(&e.id))
        .cloned()
        .collect();

    let clusters: Vec<Cluster> = map
        .clusters
        .iter()
        .filter_map(|c| {
            let element_ids: Vec<String> = c
                .element_ids
                .iter()
                .filter(|id| kept_ids.contains(*id))
                .cloned()
                .collect();
            if element_ids.is_empty() {
                None
            } else {
                let is_list = element_ids.len() >= 2;
                Some(Cluster {
                    id: c.id.clone(),
                    cluster_type: if is_list {
                        ClusterType::List
                    } else {
                        ClusterType::Single
                    },
                    element_ids,
                    prefix_signature: c.prefix_signature.clone(),
                    // Preserve CLASSIFY enrichment when the filtered cluster stays a List;
                    // drop it when filtering collapses the cluster to a Single.
                    container_role: is_list.then(|| c.container_role.clone()).flatten(),
                    child_role: is_list.then(|| c.child_role.clone()).flatten(),
                    variable_kind: is_list.then(|| c.variable_kind.clone()).flatten(),
                })
            }
        })
        .collect();

    ElementMap {
        metadata: MapMetadata {
            url: map.metadata.url.clone(),
            element_count: elements.len(),
            cluster_count: clusters.len(),
            timestamp_ms: map.metadata.timestamp_ms,
        },
        elements,
        clusters,
    }
}

pub fn build_element_map_json(
    snapshot_json: &str,
    options_json: &str,
) -> Result<String, CoreError> {
    let snapshot: DOMSnapshot = serde_json::from_str(snapshot_json)?;
    let options: MapOptions = if options_json.trim().is_empty() {
        MapOptions::default()
    } else {
        serde_json::from_str(options_json)?
    };
    let map = build_element_map(&snapshot, &options);
    Ok(serde_json::to_string(&map)?)
}

pub fn filter_element_map_json(map_json: &str, spec_json: &str) -> Result<String, CoreError> {
    let map: ElementMap = serde_json::from_str(map_json)?;
    let spec: FilterSpec = serde_json::from_str(spec_json)?;
    let filtered = filter_element_map(&map, &spec);
    Ok(serde_json::to_string(&filtered)?)
}

fn is_interactive(element: &DOMElementInfo) -> bool {
    is_interactive_tag(&element.tag)
}

fn is_interactive_tag(tag: &str) -> bool {
    INTERACTIVE_TAGS.contains(&tag.to_lowercase().as_str())
}

fn escape_css_attr(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn card_snapshot() -> DOMSnapshot {
        DOMSnapshot {
            html: "<div class=\"grid\">...</div>".to_string(),
            elements: vec![
                DOMElementInfo {
                    selector: "[data-testid=\"card-1\"]".to_string(),
                    tag: "article".to_string(),
                    attributes: [("data-testid".to_string(), "card-1".to_string())].into(),
                    text_content: Some("Card 1".to_string()),
                    path: vec!["div:-".to_string(), "article:-".to_string()],
                    position_in_parent: Some(0),
                    visible: None,
                    computed_role: None,
                    accessible_name: None,
                    scope_hint: None,
                },
                DOMElementInfo {
                    selector: "[data-testid=\"card-2\"]".to_string(),
                    tag: "article".to_string(),
                    attributes: [("data-testid".to_string(), "card-2".to_string())].into(),
                    text_content: Some("Card 2".to_string()),
                    path: vec!["div:-".to_string(), "article:-".to_string()],
                    position_in_parent: Some(1),
                    visible: None,
                    computed_role: None,
                    accessible_name: None,
                    scope_hint: None,
                },
                DOMElementInfo {
                    selector: "button.buy".to_string(),
                    tag: "button".to_string(),
                    attributes: HashMap::new(),
                    text_content: Some("Buy".to_string()),
                    path: vec!["button:-".to_string()],
                    position_in_parent: None,
                    visible: None,
                    computed_role: None,
                    accessible_name: None,
                    scope_hint: None,
                },
            ],
        }
    }

    #[test]
    fn build_element_map_groups_similar_paths() {
        let map = build_element_map(
            &card_snapshot(),
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );
        assert_eq!(map.elements.len(), 3);
        let list_clusters: Vec<_> = map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::List)
            .collect();
        assert!(
            !list_clusters.is_empty(),
            "expected at least one list cluster for similar articles"
        );
    }

    fn el(tag: &str, selector: &str, attrs: &[(&str, &str)], text: Option<&str>) -> DOMElementInfo {
        DOMElementInfo {
            selector: selector.to_string(),
            tag: tag.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            text_content: text.map(|t| t.to_string()),
            path: vec![format!("{tag}:-")],
            position_in_parent: None,
            visible: None,
            computed_role: None,
            accessible_name: None,
            scope_hint: None,
        }
    }

    /// Like `el`, but lets a test set the collector a11y fields (role / accessible name /
    /// scope_hint) and an explicit DOM `path` so structural similarity can discriminate.
    #[allow(clippy::too_many_arguments)]
    fn el_a11y(
        tag: &str,
        selector: &str,
        attrs: &[(&str, &str)],
        text: Option<&str>,
        path: &[&str],
        role: Option<&str>,
        name: Option<&str>,
        scope: Option<&str>,
    ) -> DOMElementInfo {
        DOMElementInfo {
            selector: selector.to_string(),
            tag: tag.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            text_content: text.map(|t| t.to_string()),
            path: path.iter().map(|s| s.to_string()).collect(),
            position_in_parent: None,
            visible: Some(true),
            computed_role: role.map(|s| s.to_string()),
            accessible_name: name.map(|s| s.to_string()),
            scope_hint: scope.map(|s| s.to_string()),
        }
    }

    #[test]
    fn cluster_exposes_variable_and_constant_split() {
        // A repeated list of cards under a shared `listitem` container: every member is an
        // <a role=link> within `card-list` (the CONSTANT container signal), differing only by
        // its visible text/name (the VARIABLE discriminator). The List cluster must surface
        // variable_kind="text", a child_role of "link", and the shared container role.
        let snapshot = DOMSnapshot {
            html: "<ul class=\"card-list\">...</ul>".to_string(),
            elements: vec![
                el_a11y(
                    "a",
                    "li:nth-of-type(1) > a",
                    &[("href", "/products/1")],
                    Some("Phone"),
                    &["ul:list", "li:listitem", "a:link"],
                    Some("link"),
                    Some("Phone"),
                    Some("listitem"),
                ),
                el_a11y(
                    "a",
                    "li:nth-of-type(2) > a",
                    &[("href", "/products/2")],
                    Some("Tablet"),
                    &["ul:list", "li:listitem", "a:link"],
                    Some("link"),
                    Some("Tablet"),
                    Some("listitem"),
                ),
                el_a11y(
                    "a",
                    "li:nth-of-type(3) > a",
                    &[("href", "/products/3")],
                    Some("Laptop"),
                    &["ul:list", "li:listitem", "a:link"],
                    Some("link"),
                    Some("Laptop"),
                    Some("listitem"),
                ),
            ],
        };

        let map = build_element_map(
            &snapshot,
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );

        let list = map
            .clusters
            .iter()
            .find(|c| c.cluster_type == ClusterType::List)
            .expect("repeated cards must form one List cluster");
        assert_eq!(list.element_ids.len(), 3, "all three cards share the list");

        // VARIABLE: the visible text differs across members.
        assert_eq!(
            list.variable_kind.as_deref(),
            Some("text"),
            "differing accessible name must be the text discriminator: {list:?}"
        );
        // CHILD role: the interactive member itself.
        assert_eq!(
            list.child_role.as_deref(),
            Some("link"),
            "child role must be the member's computed role: {list:?}"
        );
        // CONSTANT: the container (scope_hint) is shared by all members.
        assert_eq!(
            list.container_role.as_deref(),
            Some("listitem"),
            "shared container scope must surface as container_role: {list:?}"
        );
    }

    #[test]
    fn container_role_derived_from_li_parent_path() {
        // A bare `<ul><li><a>…</a></li>…</ul>` list with NO landmark ancestor: the collector
        // sets no scope_hint (landmarks only). The container role must therefore be recovered
        // from the members' DOM `path` — each member's parent is `<li>`, so the repeating
        // unit's role is `listitem`. Without this the generator would fall back to `.nth()`.
        let path = &["ul:-", "li:-", "a:-"][..];
        let snapshot = DOMSnapshot {
            html: "<ul><li><a>…</a></li></ul>".to_string(),
            elements: vec![
                el_a11y(
                    "a",
                    "li:nth-of-type(1) > a",
                    &[],
                    Some("Home"),
                    path,
                    Some("link"),
                    Some("Home"),
                    None, // no scope_hint — bare list, no landmark
                ),
                el_a11y(
                    "a",
                    "li:nth-of-type(2) > a",
                    &[],
                    Some("About"),
                    path,
                    Some("link"),
                    Some("About"),
                    None,
                ),
                el_a11y(
                    "a",
                    "li:nth-of-type(3) > a",
                    &[],
                    Some("Contact"),
                    path,
                    Some("link"),
                    Some("Contact"),
                    None,
                ),
            ],
        };

        let map = build_element_map(
            &snapshot,
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );

        let list = map
            .clusters
            .iter()
            .find(|c| c.cluster_type == ClusterType::List)
            .expect("repeated bare-list links must form one List cluster");
        assert_eq!(list.element_ids.len(), 3, "all three links share the list");

        // container_role recovered from the `<li>` parent in the path, not from scope_hint.
        assert_eq!(
            list.container_role.as_deref(),
            Some("listitem"),
            "li parent in path must surface listitem as container_role: {list:?}"
        );
        // child_role is the member's own role.
        assert_eq!(
            list.child_role.as_deref(),
            Some("link"),
            "child role must be the member's computed role: {list:?}"
        );
        // variable_kind is the differing visible text.
        assert_eq!(
            list.variable_kind.as_deref(),
            Some("text"),
            "differing accessible name must be the text discriminator: {list:?}"
        );
    }

    #[test]
    fn classify_reconciles_semantickey_with_structural_similarity() {
        // Two flavours of <a role=link> with no href and the same coarse text shape ("L D"):
        // they share a SemanticKey. But the FIRST pair sits at the SAME DOM depth/path (a real
        // repeated list), while the heterogeneous members are scattered at wildly different
        // depths (deep nav vs shallow footer) — structurally dissimilar despite the shared key.
        //
        // Expectation: the genuine homogeneous pair forms ONE List (SemanticKey + similarity
        // agree); the structurally heterogeneous members do NOT fuse into a List — low pairwise
        // similarity dissolves them into Singles.
        let homogeneous_path = &["nav:-", "ul:list", "li:listitem", "a:link"][..];

        // Real repeated component: identical structural path, differing text.
        let card_a = el_a11y(
            "a",
            "nav > ul > li:nth-of-type(1) > a",
            &[],
            Some("Item 1"),
            homogeneous_path,
            Some("link"),
            Some("Item 1"),
            Some("listitem"),
        );
        let card_b = el_a11y(
            "a",
            "nav > ul > li:nth-of-type(2) > a",
            &[],
            Some("Item 2"),
            homogeneous_path,
            Some("link"),
            Some("Item 2"),
            Some("listitem"),
        );

        // Heterogeneous pair: same SemanticKey (role=link, no href, "L D" text shape) but very
        // different structural paths -> low pairwise similarity -> must NOT become a List.
        let scattered_deep = el_a11y(
            "a",
            "header > div > div > div > section > nav > ul > li > span > a",
            &[],
            Some("Tab 9"),
            &[
                "header:-",
                "div:-",
                "div:-",
                "div:-",
                "section:-",
                "nav:-",
                "ul:-",
                "li:-",
                "span:-",
                "a:link",
            ],
            Some("link"),
            Some("Tab 9"),
            Some("header"),
        );
        let scattered_shallow = el_a11y(
            "a",
            "footer > a",
            &[],
            Some("Tab 8"),
            &["footer:-", "a:link"],
            Some("link"),
            Some("Tab 8"),
            Some("footer"),
        );

        // Homogeneous pair on its own -> exactly one List.
        let homo_map = build_element_map(
            &DOMSnapshot {
                html: String::new(),
                elements: vec![card_a.clone(), card_b.clone()],
            },
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );
        let homo_lists = homo_map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::List)
            .count();
        assert_eq!(
            homo_lists, 1,
            "structurally identical repeated cards must form one List: {:?}",
            homo_map.clusters
        );

        // Structurally heterogeneous members sharing the SemanticKey -> NOT a List.
        let hetero_map = build_element_map(
            &DOMSnapshot {
                html: String::new(),
                elements: vec![scattered_deep, scattered_shallow],
            },
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );
        let hetero_lists = hetero_map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::List)
            .count();
        assert_eq!(
            hetero_lists, 0,
            "structurally heterogeneous same-key links must dissolve into Singles: {:?}",
            hetero_map.clusters
        );
    }

    #[test]
    fn cascade_picks_href_when_unique() {
        let elements = vec![
            el("a", "a.home", &[("href", "/home")], Some("Home")),
            el("a", "a.about", &[("href", "/about")], Some("About")),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(rec.strategy, "href");
        assert_eq!(rec.selector, "[href=\"/home\"]");
        assert!(rec.confidence >= 0.8, "confidence was {}", rec.confidence);
    }

    #[test]
    fn cascade_falls_to_positional_when_no_signal() {
        let elements = vec![
            el("div", "div.a", &[], None),
            el("span", "span.b", &[], None),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(rec.strategy, "css");
        assert_eq!(rec.selector, "div.a");
        assert!(rec.confidence < 0.5, "confidence was {}", rec.confidence);
    }

    #[test]
    fn confidence_drops_for_non_unique_candidate() {
        // Two anchors share the same href -> href candidate is not unique.
        let elements = vec![
            el("a", "a.first", &[("href", "/dup")], Some("First")),
            el("a", "a.second", &[("href", "/dup")], Some("Second")),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        // The shared href is no longer a unique candidate; the cascade must move on
        // (to the unique text/positional candidate) rather than recommend a colliding href.
        assert_ne!(
            rec.strategy, "href",
            "non-unique href must not win, got {rec:?}"
        );
    }

    #[test]
    fn recommend_locator_single_element_has_degraded_confidence() {
        // Single-element API has no uniqueness data: confidence == strategy base score.
        let element = el("a", "a.home", &[("href", "/home")], Some("Home"));
        let rec = recommend_locator(&element);
        assert_eq!(rec.strategy, "href");
        assert!(
            (rec.confidence - BASE_HREF).abs() < 1e-9,
            "expected base href confidence, got {}",
            rec.confidence
        );
    }

    #[test]
    fn cluster_splits_heterogeneous_divsoup() {
        // Div-soup SPA: many nodes share a tag prefix but are semantically unrelated
        // (different tags, different/no href, structurally different text). They must NOT
        // collapse into one mega-`List`; each heterogeneous node stays `Single`.
        let snapshot = DOMSnapshot {
            html: "<div id=\"app\">...</div>".to_string(),
            elements: vec![
                el("a", "a.logo", &[("href", "/")], Some("Brand")),
                el("a", "a.cart", &[("href", "/cart")], Some("Cart 3 items")),
                el("button", "button.menu", &[], Some("Open menu")),
                el("input", "input.search", &[("placeholder", "Search")], None),
                el("a", "a.help", &[("href", "/help/faq")], Some("FAQ")),
                el("button", "button.x", &[], None),
            ],
        };

        let map = build_element_map(
            &snapshot,
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );

        assert_eq!(map.elements.len(), 6);

        // No mega-cluster: nothing groups more than a couple of nodes, and the largest
        // cluster is far smaller than the element count.
        let max_cluster = map
            .clusters
            .iter()
            .map(|c| c.element_ids.len())
            .max()
            .unwrap_or(0);
        assert!(
            max_cluster <= 2,
            "expected no mega-cluster, largest was {max_cluster}: {:?}",
            map.clusters
        );

        // Heterogeneous nodes are `Single`, not fused into one list.
        let list_clusters = map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::List)
            .count();
        assert!(
            list_clusters == 0,
            "heterogeneous div-soup must not form list clusters: {:?}",
            map.clusters
        );
    }

    #[test]
    fn cluster_groups_repeated_cards_into_single_list() {
        // Genuine repeated cards: same tag + href of one pattern + same text shape.
        let snapshot = DOMSnapshot {
            html: "<ul>...</ul>".to_string(),
            elements: vec![
                el("a", "a.p1", &[("href", "/products/1")], Some("Product 1")),
                el("a", "a.p2", &[("href", "/products/2")], Some("Product 2")),
                el("a", "a.p3", &[("href", "/products/3")], Some("Product 3")),
                // A lone, unrelated link must stay out of the products list.
                el("a", "a.about", &[("href", "/about")], Some("About us")),
            ],
        };

        let map = build_element_map(
            &snapshot,
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );

        let list_clusters: Vec<_> = map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::List)
            .collect();
        assert_eq!(
            list_clusters.len(),
            1,
            "expected exactly one product list cluster: {:?}",
            map.clusters
        );
        assert_eq!(
            list_clusters[0].element_ids.len(),
            3,
            "the three product cards must share one list cluster"
        );

        // The unrelated link is its own `Single`, not merged into the products list.
        let singles = map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::Single)
            .count();
        assert_eq!(
            singles, 1,
            "the about link must remain Single: {:?}",
            map.clusters
        );
    }

    #[test]
    fn cluster_unique_hrefs_not_collapsed_into_list() {
        // A nav block of links with DIFFERENT destinations (each a unique href) but a similar
        // text shape. Under the old first-segment-only key they all shared `/ru/...` and the
        // coarse `L` text shape, collapsing into one mega-`List` that the cluster method then
        // addressed with a broken positional `.nth()`. They must now stay individually
        // addressable: no list cluster fuses these distinct routes.
        let snapshot = DOMSnapshot {
            html: "<nav>...</nav>".to_string(),
            elements: vec![
                el("a", "a.person", &[("href", "/ru/person")], Some("Person")),
                el(
                    "a",
                    "a.business",
                    &[("href", "/ru/business")],
                    Some("Business"),
                ),
                el("a", "a.about", &[("href", "/ru/about")], Some("About")),
                el("a", "a.legal", &[("href", "/ru/legal")], Some("Legal")),
            ],
        };

        let map = build_element_map(
            &snapshot,
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );

        assert_eq!(map.elements.len(), 4);

        let list_clusters: Vec<_> = map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::List)
            .collect();
        assert!(
            list_clusters.is_empty(),
            "distinct-href links must not fuse into a list: {:?}",
            map.clusters
        );

        // Every distinct route is its own `Single`.
        let singles = map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::Single)
            .count();
        assert_eq!(
            singles, 4,
            "each unique link must remain a Single: {:?}",
            map.clusters
        );
    }

    #[test]
    fn cluster_only_real_repeats_form_list() {
        // Genuinely repeated cards: a templated href path (`/products/<id>`) plus a shared
        // structural text shape. The numeric id segment is masked to `*`, so all three share
        // the `products/*` template and collapse into ONE list of size 3 — the correct
        // repeated-component case (addressable as a list), unlike distinct routes.
        let snapshot = DOMSnapshot {
            html: "<ul>...</ul>".to_string(),
            elements: vec![
                el(
                    "a",
                    "a.i1",
                    &[("href", "/catalog/items/101")],
                    Some("Item 101"),
                ),
                el(
                    "a",
                    "a.i2",
                    &[("href", "/catalog/items/102")],
                    Some("Item 102"),
                ),
                el(
                    "a",
                    "a.i3",
                    &[("href", "/catalog/items/103")],
                    Some("Item 103"),
                ),
                el(
                    "a",
                    "a.i4",
                    &[("href", "/catalog/items/104")],
                    Some("Item 104"),
                ),
            ],
        };

        let map = build_element_map(
            &snapshot,
            &MapOptions {
                include_non_interactive: true,
                ..Default::default()
            },
        );

        let list_clusters: Vec<_> = map
            .clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::List)
            .collect();
        assert_eq!(
            list_clusters.len(),
            1,
            "the templated repeats must form exactly one list: {:?}",
            map.clusters
        );
        assert_eq!(
            list_clusters[0].element_ids.len(),
            4,
            "all four templated repeats share one list cluster"
        );
    }

    #[test]
    fn cascade_role_name_rescues_unique_text_without_attrs() {
        // An <a> with no id/testid/href but a unique text. The bare text is NOT globally
        // unique (a <span> elsewhere repeats it), so the `text` tier cannot win on
        // uniqueness — but (role=link, name="Sign in") IS unique, so the role+name tier must
        // rescue it into a semantic `role` locator instead of positional CSS.
        let elements = vec![
            el(
                "a",
                "nav > ul > li:nth-of-type(3) > a:nth-of-type(1)",
                &[],
                Some("Sign in"),
            ),
            // Duplicate bare text in a different tag (not a link) -> text not globally unique,
            // but it is NOT an <a>, so (link, "Sign in") stays unique.
            el("span", "footer > span:nth-of-type(2)", &[], Some("Sign in")),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(rec.strategy, "role", "expected role rescue, got {rec:?}");
        assert_eq!(rec.value.as_deref(), Some("Sign in"));
        assert_ne!(rec.strategy, "css", "must not fall to positional css");
        assert!(
            rec.confidence >= 0.8,
            "role+name confidence should be high, got {}",
            rec.confidence
        );
    }

    #[test]
    fn anchored_css_fallback_shorter_than_full_nth_chain() {
        // No-signal element with a stable ancestor token (`li.product-card`) in its own
        // selector: the positional fallback must anchor on it, dropping the long prefix.
        let full_chain =
            "html > body > div#root > main > ul > li.product-card > div > a:nth-of-type(2)";
        let elements = vec![el("a", full_chain, &[], None)];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(rec.strategy, "css");
        assert!(
            rec.selector.len() < full_chain.len(),
            "anchored fallback should be shorter than the full chain: {}",
            rec.selector
        );
        assert_eq!(
            rec.selector, "li.product-card > div > a:nth-of-type(2)",
            "expected anchor on the nearest stable token"
        );
        // No stable token anywhere -> selector is returned unchanged (no over-trimming).
        let bare = "html > body > div > div > a:nth-of-type(2)";
        let bare_elements = vec![el("a", bare, &[], None)];
        let bare_idx = SnapshotIndexes::build(&bare_elements);
        let bare_rec = recommend_locator_in_snapshot(&bare_elements[0], &bare_idx);
        assert_eq!(bare_rec.selector, bare, "no stable token -> unchanged");
    }

    #[test]
    fn filter_interactive_only() {
        let map = build_element_map(&card_snapshot(), &MapOptions::default());
        let filtered = filter_element_map(
            &map,
            &FilterSpec {
                interactive_only: true,
                ..Default::default()
            },
        );
        assert_eq!(filtered.elements.len(), 1);
        assert_eq!(filtered.elements[0].tag, "button");
    }

    #[test]
    fn finalize_prefers_max_base_among_unique() {
        // An anchor with a unique href AND a unique (role=link, name) pair. The old "first unique
        // in insertion order" logic would pick whichever tier appeared first; the new max-base
        // rule must pick `href` (0.92) over `role` (0.90) because both are unique.
        let elements = vec![
            el("a", "a.home", &[("href", "/home")], Some("Home")),
            el("a", "a.about", &[("href", "/about")], Some("About")),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(
            rec.strategy, "href",
            "max-base among unique must pick href over role: {rec:?}"
        );
        assert_eq!(rec.match_count, Some(1), "unique locator reports 1 match");
    }

    #[test]
    fn role_first_button_with_text_uses_role() {
        // A bare <button> with visible text but no attributes: its bare text is unique AND the
        // (role=button, name) pair is unique. role (0.90) must outrank text (0.80).
        let elements = vec![
            el("button", "button.save", &[], Some("Save changes")),
            el("button", "button.cancel", &[], Some("Cancel")),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(
            rec.strategy, "role",
            "unique role+name must beat unique text: {rec:?}"
        );
        assert_eq!(rec.value.as_deref(), Some("Save changes"));
    }

    #[test]
    fn named_candidate_outranks_positional_css() {
        // A non-unique role+name (two identical buttons, no scope to disambiguate) has NO unique
        // named candidate, but the named candidate (even penalised) must still beat the positional
        // css fallback — never degrade to css while a named handle exists.
        let elements = vec![
            el("button", "div > button:nth-of-type(1)", &[], Some("More")),
            el("button", "div > button:nth-of-type(2)", &[], Some("More")),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_ne!(
            rec.strategy, "css",
            "a named candidate must outrank positional css: {rec:?}"
        );
        // Non-unique -> match_count surfaces the collision so the generator can add .first().
        assert_eq!(
            rec.match_count,
            Some(2),
            "non-unique reports the count: {rec:?}"
        );
    }

    #[test]
    fn hash_href_treated_as_no_href() {
        // An <a href="#"> is a no-op anchor: no href candidate is generated, so the cascade falls
        // through to the (role=link, name) handle instead of a useless `[href="#"]`.
        let elements = vec![
            el("a", "a.toggle", &[("href", "#")], Some("Toggle menu")),
            el("a", "a.other", &[("href", "/real")], Some("Real")),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_ne!(
            rec.strategy, "href",
            "hash href must not be a candidate: {rec:?}"
        );
        assert_eq!(rec.value.as_deref(), Some("Toggle menu"));
    }

    #[test]
    fn scoped_uniqueness_resolves_within_container() {
        // Two "Edit" links: globally the (role=link, name="Edit") pair is NOT unique, but each
        // lives in a DIFFERENT scope_hint. The scoped (scope, role, name) tuple is unique, so the
        // cascade emits a scoped role candidate carrying `scope` and `filter_text`.
        let elements = vec![
            el_a11y(
                "a",
                "section#row-1 > a",
                &[],
                Some("Edit"),
                &["section:-", "a:link"],
                Some("link"),
                Some("Edit"),
                Some("row-1"),
            ),
            el_a11y(
                "a",
                "section#row-2 > a",
                &[],
                Some("Edit"),
                &["section:-", "a:link"],
                Some("link"),
                Some("Edit"),
                Some("row-2"),
            ),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(
            rec.strategy, "role",
            "scoped role candidate expected: {rec:?}"
        );
        assert_eq!(
            rec.scope.as_deref(),
            Some("row-1"),
            "scope must carry the container scope_hint: {rec:?}"
        );
        assert_eq!(
            rec.filter_text.as_deref(),
            Some("Edit"),
            "filter_text must carry the per-member discriminator: {rec:?}"
        );
    }

    #[test]
    fn verify_rejects_globally_ambiguous_scoped_locator() {
        // Three "Edit" links: two share the SAME scope_hint ("row-1"). The (scope, role, name)
        // tuple for that scope matches 2 elements, so VERIFY must REJECT the scoped candidate
        // (not globally unambiguous) and degrade — no `scope` is set on the recommendation.
        let elements = vec![
            el_a11y(
                "a",
                "section#row-1 > a:nth-of-type(1)",
                &[],
                Some("Edit"),
                &["section:-", "a:link"],
                Some("link"),
                Some("Edit"),
                Some("row-1"),
            ),
            el_a11y(
                "a",
                "section#row-1 > a:nth-of-type(2)",
                &[],
                Some("Edit"),
                &["section:-", "a:link"],
                Some("link"),
                Some("Edit"),
                Some("row-1"),
            ),
            el_a11y(
                "a",
                "section#row-2 > a",
                &[],
                Some("Edit"),
                &["section:-", "a:link"],
                Some("link"),
                Some("Edit"),
                Some("row-2"),
            ),
        ];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(
            rec.scope, None,
            "ambiguous scope (2 matches) must NOT emit a scoped locator: {rec:?}"
        );
    }

    #[test]
    fn uniqueness_computed_among_visible_only() {
        // Two anchors share the same href, but the colliding one is hidden (visible=false). For a
        // user the href is effectively unique, so it must NOT be demoted by the hidden duplicate:
        // the visible element keeps the unique href locator.
        let mut hidden = el("a", "a.dup-hidden", &[("href", "/dup")], Some("Hidden"));
        hidden.visible = Some(false);
        let visible = el("a", "a.dup-visible", &[("href", "/dup")], Some("Visible"));
        let elements = vec![visible, hidden];
        let indexes = SnapshotIndexes::build(&elements);
        let rec = recommend_locator_in_snapshot(&elements[0], &indexes);
        assert_eq!(
            rec.strategy, "href",
            "href colliding only with a hidden element stays unique: {rec:?}"
        );
        assert_eq!(
            rec.match_count,
            Some(1),
            "hidden duplicate must not inflate count: {rec:?}"
        );
    }
}

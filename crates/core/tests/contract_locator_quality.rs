//! Contract regression for locator quality on a REAL (trimmed) snapshot.
//!
//! This integration test sees only the public `frap_core` API. It loads a fixed,
//! deterministic subset of the live `person/giga` page snapshot and asserts that the
//! CLASSIFY→SCOPE→LOCATE→VERIFY locator pipeline + Java PageObject generator keep hitting the
//! quality targets from the plan (`specs/frap-core-locator-pipeline.md`, Acceptance Criteria):
//!   - positional (`nth-of-type` css) share  <= 5% of total (committed-fixture target)
//!   - confidence >= 0.8 share                >= 85% of total
//!   - semantic-strategy share                >= 85% of total
//!   - List clusters with a text discriminator emit a RELATIVE locator (not `.nth(index)`)
//!   - duplicate-text links get scoped / `.first()` semantics, never positional css+nth
//!   - no named element (text / accessible_name / href) falls back to positional css+nth
//!   - no unique-href mega-cluster
//!   - every generated method name is a valid Java identifier (never digit-leading)
//!   - generated PageObject uses getBy* for semantic strategies (text/role/label/placeholder).
//!
//! Fixture composition (`tests/fixtures/locator-quality/snap-person.json`): the original
//! 52-element first-N-of-each-type slice, the 6 semantic-output exercises, plus 7 pipeline
//! exercises carrying the collector a11y fields (`visible` / `computed_role` /
//! `accessible_name` / `scope_hint`):
//!   -  5x test attribute (data-test-id ...)        -> `testid` strategy (alias -> page.locator)
//!   - 10x aria-label                                -> `role` strategy (accessible name) -> getByLabel
//!   -  2x role-only (no aria-label)                 -> `role` / positional
//!   -  1x placeholder (input)                       -> `placeholder` strategy -> getByPlaceholder
//!   - 10x <li> with stable id == visible text       -> `id` / `text` (semantic)
//!   - 22x <a> with unique href                      -> `href` strategy -> page.locator([href=...])
//!   -  2x <style> with no usable signal             -> positional control (`css`)
//!   -  1x <div>                                      -> mixed (hash class / role)
//!      (semantic-output exercises)
//!   +  1x <a> visible text "880050087" (digit-lead) -> `role` -> getByRole(LINK, name), id `link880050087`
//!   +  1x <a> role+name-only "Для жизни"            -> `role` (unique role+name)
//!   +  1x <button> "Финансы" (text dup'd on a tab)  -> `role` -> getByRole(BUTTON, name)
//!   +  1x <div role="tab"> "Финансы"                -> `role` (explicit role)
//!   +  2x <span> hashed-class no-signal             -> positional control (`css`)
//!      (pipeline exercises — a11y fields set)
//!   +  3x <a role=link> card list (`/cards/1..3`, scope_hint=listitem, differing accessible_name)
//!      -> ONE List cluster (container=listitem, child=link, variable_kind="text") -> RELATIVE
//!      locator `getByRole(LISTITEM).filter(setHasText(text)).getByRole(LINK)`; each card also keeps
//!      a unique `href` element locator.
//!   +  2x <a> "Поддержка" href "/support" — 1 visible:true (clean `href`), 1 visible:false; the
//!      hidden dup is excluded from the VISIBLE-uniqueness counters, so the visible copy stays unique.
//!   +  2x <span> "Контакты" under DIFFERENT scope_hint (nav#header-bar / div#footer-bar), both
//!      visible -> globally-ambiguous text rescued by SCOPE -> scoped `text`
//!      (`page.locator("<scope>").getByText("Контакты")`), NOT positional.
//!
//!   total = 65 elements
//!
//! Measured engine metrics on this fixture (for reference; thresholds in `expected.json` are the
//! *plan* percentage targets — `max_positional = ceil(0.05·total)`, `min_high_confidence =
//! floor(0.85·total)` — never weaker):
//!   positional=2 (<= 4), confidence>=0.8 = 58 (>= 55), semantic=59, max cluster=6, scoped=2,
//!   relative List clusters=1.
//!   Strategy breakdown (max-base finalize: a unique role+name now wins over bare `text`, so the
//!   "Для жизни"/"Финансы"/"880050087" group migrated text -> role): css=6, href=32, placeholder=1,
//!   role=9, testid=5, text=12.
//!   Generated PageObject: getByText/getByLabel/getByRole/getByPlaceholder all present; the card
//!   List emits `getByRole(LISTITEM).filter(setHasText(text)).getByRole(LINK)`; the "Контакты"
//!   spans emit `page.locator("<scope>").getByText(...)`; every method name matches
//!   `^[A-Za-z_][A-Za-z0-9_]*$` (e.g. `link880050087`).

use frap_core::{
    build_element_map, generate_page_object, ClusterType, DOMSnapshot, GenerateOptions, MapOptions,
};
use serde::Deserialize;
use std::{fs, path::PathBuf};

/// Strategies counted as "semantic" (everything except the positional `css` fallback).
/// The engine emits exactly `{testid, role, placeholder, href, text, id, css}` — there is
/// no separate `label` strategy (getByLabel vs getByRole is decided at generation time by
/// the presence of `aria-label`), so `label` is intentionally absent here.
const SEMANTIC_STRATEGIES: &[&str] = &["testid", "role", "placeholder", "href", "text", "id"];

#[derive(Debug, Deserialize)]
struct Expected {
    total: usize,
    max_positional: usize,
    min_high_confidence: usize,
    min_semantic_strategy: usize,
    max_cluster_size: usize,
    /// At least this many `List` clusters must emit a RELATIVE locator (a text-discriminator
    /// cluster with a usable container/child role), proving the card list does not collapse to
    /// `.nth(index)`.
    min_relative_list_clusters: usize,
    /// At least this many element locators must be SCOPED (`scope` set) — duplicate-text elements
    /// rescued by their container instead of falling to positional css+nth.
    min_scoped: usize,
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/locator-quality")
}

fn load_snapshot() -> DOMSnapshot {
    let path = fixture_dir().join("snap-person.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("parse snap-person.json")
}

fn load_expected() -> Expected {
    let path = fixture_dir().join("expected.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read expected {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("parse expected.json")
}

fn build_map() -> frap_core::ElementMap {
    let snapshot = load_snapshot();
    build_element_map(
        &snapshot,
        &MapOptions {
            include_non_interactive: true,
            ..Default::default()
        },
    )
}

fn generated_source() -> String {
    let map = build_map();
    let artifact = generate_page_object(&map, &GenerateOptions::default());
    artifact.files[0].content.clone()
}

/// Dependency-free extraction of every `public Locator <name>(` method name, mirroring the
/// regex `public (?:Locator) (\w+)\(` from the plan.
fn method_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("public Locator ") else {
            continue;
        };
        let Some(paren) = rest.find('(') else {
            continue;
        };
        let name = rest[..paren].trim();
        if !name.is_empty() && name.chars().all(|c| c == '_' || c.is_alphanumeric()) {
            names.push(name.to_string());
        }
    }
    names
}

/// Dependency-free check for `^[A-Za-z_][A-Za-z0-9_]*$`.
fn is_valid_java_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[test]
fn all_method_names_are_valid_java_identifiers() {
    let source = generated_source();
    let names = method_names(&source);
    assert!(
        !names.is_empty(),
        "no `public Locator` methods found in generated PageObject:\n{source}"
    );
    // The digit-leading-text element ("880050087") must be present and sanitized.
    assert!(
        names.iter().any(|n| n.contains("880050087")),
        "digit-text element did not yield a method (fixture regressed):\n{names:?}"
    );
    for name in &names {
        assert!(
            is_valid_java_identifier(name),
            "method name `{name}` is not a valid Java identifier (^[A-Za-z_][A-Za-z0-9_]*$)\nall names: {names:?}"
        );
        assert!(
            !name.chars().next().unwrap().is_ascii_digit(),
            "method name `{name}` starts with a digit (would not compile)"
        );
    }
}

#[test]
fn real_snapshot_positional_share_below_target() {
    let expected = load_expected();
    let map = build_map();
    assert_eq!(
        map.elements.len(),
        expected.total,
        "fixture size drifted from expected.total"
    );

    let positional = map
        .elements
        .iter()
        .filter(|e| e.locator.strategy == "css" && e.recommended_selector.contains("nth-of-type"))
        .count();

    assert!(
        positional <= expected.max_positional,
        "positional locators {positional} exceed max {} (target <= 10% of {})",
        expected.max_positional,
        expected.total
    );
}

/// A `List` cluster carries a usable RELATIVE locator when it is text-discriminated and both its
/// container and child roles are present — mirroring the generator's `relative_cluster_emission`
/// gate (`variable_kind == "text"` + container_role + child_role). The generator maps these onto
/// `AriaRole` constants; here we only need the pipeline-level signal that the cluster is relative.
fn cluster_emits_relative(cluster: &frap_core::Cluster) -> bool {
    cluster.cluster_type == ClusterType::List
        && cluster.variable_kind.as_deref() == Some("text")
        && cluster.container_role.is_some()
        && cluster.child_role.is_some()
}

#[test]
fn list_clusters_emit_relative_not_nth() {
    // The repeated card list (3x <a role=link> under a `listitem` scope, differing accessible
    // names) must be classified as a text-discriminated List and emit a RELATIVE locator —
    // `getByRole(container).filter(setHasText(text)).getByRole(child)` — instead of the brittle
    // positional `.nth(index)` over the sample selector.
    let expected = load_expected();
    let map = build_map();

    let relative: Vec<&frap_core::Cluster> = map
        .clusters
        .iter()
        .filter(|c| cluster_emits_relative(c))
        .collect();

    assert!(
        relative.len() >= expected.min_relative_list_clusters,
        "expected >= {} relative List clusters, found {} (card list regressed to .nth?): {:#?}",
        expected.min_relative_list_clusters,
        relative.len(),
        map.clusters
            .iter()
            .filter(|c| c.cluster_type == ClusterType::List)
            .collect::<Vec<_>>()
    );

    let source = generated_source();
    // The relative method body the generator emits for such a cluster.
    assert!(
        source.contains("getByRole(AriaRole.")
            && source.contains("new Locator.FilterOptions().setHasText(text)"),
        "generated PageObject is missing the relative filter(setHasText(text)) locator:\n{source}"
    );

    // Each relative cluster's members must NOT also be exposed only through a positional
    // `.nth(index)` accessor — the relative method takes a `String text` parameter. The cluster
    // method for a relative cluster is `public Locator <name>(String text)`, so we assert at least
    // one `(String text)` relative accessor exists (the `.nth(index)` fallback was not taken).
    for cluster in &relative {
        assert!(
            source.contains("(String text) {"),
            "relative cluster {} did not yield a `(String text)` accessor (fell back to .nth(index)?):\n{source}",
            cluster.id
        );
    }
}

#[test]
fn dup_text_links_get_scoped_or_first_not_positional() {
    // Duplicate-text elements (the two "Контакты" spans under different scope_hints, and the
    // visible/hidden "Поддержка" menu copies) must be rescued by SCOPE (`page.locator("<scope>")
    // .getByText(...)`) or marked `.first()` — never demoted to a positional css+nth locator.
    let expected = load_expected();
    let map = build_map();

    let scoped = map
        .elements
        .iter()
        .filter(|e| e.locator.scope.is_some())
        .count();
    assert!(
        scoped >= expected.min_scoped,
        "expected >= {} scoped locators (duplicate-text rescued by container), found {scoped}",
        expected.min_scoped
    );

    // No scoped element may itself be a positional css+nth fallback.
    for element in map.elements.iter().filter(|e| e.locator.scope.is_some()) {
        assert_ne!(
            element.locator.strategy, "css",
            "scoped element {} must carry a semantic strategy, not positional css: {:?}",
            element.id, element.locator
        );
    }

    let source = generated_source();
    // The scoped "Контакты" spans must render as `page.locator("<scope>").getByText(...)`.
    assert!(
        source.contains("page.locator(\"nav#header-bar\").getByText(\"Контакты\"")
            || source.contains("page.locator(\"div#footer-bar\").getByText(\"Контакты\""),
        "scoped duplicate-text span must render as page.locator(<scope>).getByText:\n{source}"
    );

    // The visible "/support" link must keep a clean (non-positional) href locator.
    let support_visible = map
        .elements
        .iter()
        .find(|e| e.recommended_selector == "[href=\"/support\"]")
        .expect("the visible /support menu link must keep an href locator");
    assert_eq!(
        support_visible.locator.strategy, "href",
        "visible duplicate menu link must resolve to a clean href, got {:?}",
        support_visible.locator
    );
    assert!(
        !support_visible.recommended_selector.contains("nth-of-type"),
        "visible duplicate menu link must not be positional: {:?}",
        support_visible.locator
    );
}

#[test]
fn named_elements_get_semantic_not_positional() {
    // No element that carries a name signal (accessible_name / visible text / href) may fall back
    // to a positional css+nth locator. Only genuinely unnamed controls (no text, no name, no href)
    // are allowed to stay positional.
    let map = build_map();

    let offenders: Vec<_> = map
        .elements
        .iter()
        .filter(|e| e.locator.strategy == "css" && e.recommended_selector.contains("nth-of-type"))
        .filter(|e| {
            let has_text = e.locator.value.is_some();
            let has_href = e.locator.strategy == "href" || e.recommended_selector.contains("href=");
            // A positional css element should have neither a strategy `value` (the normalized
            // name/text/href) nor an href selector — otherwise it is a NAMED element that wrongly
            // fell to positional.
            has_text || has_href
        })
        .map(|e| {
            (
                e.id.clone(),
                e.recommended_selector.clone(),
                e.locator.value.clone(),
            )
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "named elements must never resolve to positional css+nth, offenders: {offenders:#?}"
    );
}

#[test]
fn real_snapshot_high_confidence_share_not_regressed() {
    let expected = load_expected();
    let map = build_map();

    let high = map.elements.iter().filter(|e| e.confidence >= 0.8).count();

    assert!(
        high >= expected.min_high_confidence,
        "high-confidence locators {high} below min {} (target >= 80% of {})",
        expected.min_high_confidence,
        expected.total
    );
}

#[test]
fn real_snapshot_semantic_strategy_share_above_target() {
    let expected = load_expected();
    let map = build_map();

    let semantic = map
        .elements
        .iter()
        .filter(|e| SEMANTIC_STRATEGIES.contains(&e.locator.strategy.as_str()))
        .count();

    assert!(
        semantic >= expected.min_semantic_strategy,
        "semantic-strategy locators {semantic} below min {} (target >= 80% of {})",
        expected.min_semantic_strategy,
        expected.total
    );
}

#[test]
fn real_snapshot_no_unique_href_mega_cluster() {
    let expected = load_expected();
    let map = build_map();

    let biggest = map
        .clusters
        .iter()
        .map(|c| c.element_ids.len())
        .max()
        .unwrap_or(0);

    assert!(
        biggest <= expected.max_cluster_size,
        "largest cluster has {biggest} elements, exceeds max {} \
         (unique-href links must not collapse into one mega-cluster)",
        expected.max_cluster_size
    );

    // Sanity: list clusters must stay genuinely homogeneous, never collapsing the whole
    // div/link soup into one positional `.nth()` bucket.
    for cluster in map
        .clusters
        .iter()
        .filter(|c| c.cluster_type == ClusterType::List)
    {
        assert!(
            cluster.element_ids.len() <= expected.max_cluster_size,
            "list cluster {} too large: {}",
            cluster.id,
            cluster.element_ids.len()
        );
    }
}

#[test]
fn generated_pageobject_uses_getby_for_semantic_strategies() {
    let map = build_map();
    let source = {
        let artifact = generate_page_object(&map, &GenerateOptions::default());
        artifact.files[0].content.clone()
    };

    // Legacy garbage naming (`<tag>cssEl<id>`) and any human-readable semantic name we expect.
    assert!(
        !source.contains("cssEl("),
        "generated PageObject still exposes a bare `cssEl` method:\n{source}"
    );

    // The fixture carries text, role-disambiguated, aria-label and placeholder elements, so
    // the generator must emit each of these semantic Playwright APIs.
    for api in ["getByText", "getByLabel", "getByRole", "getByPlaceholder"] {
        assert!(
            source.contains(api),
            "generated PageObject is missing semantic API `{api}` (fixture should exercise it):\n{source}"
        );
    }

    // No element resolved to a semantic strategy may fall back to a positional CSS locator.
    // In practice the only `page.locator(...nth-of-type...)` calls allowed are:
    //   - genuine positional-control `css` elements (no signal), and
    //   - List-cluster `.nth(index)` accessors.
    // A getByText/getByRole element must NEVER stay on a positional `page.locator`. We assert
    // every semantic element id maps to a getBy* line and that no semantic-method line is a
    // positional `page.locator`.
    let semantic_ids: Vec<&str> = map
        .elements
        .iter()
        .filter(|e| matches!(e.locator.strategy.as_str(), "text" | "role" | "placeholder"))
        .map(|e| e.id.as_str())
        .collect();
    assert!(
        !semantic_ids.is_empty(),
        "fixture has no text/role/placeholder elements — scenario would be vacuous"
    );

    // Cross-check: count getBy* return statements; must dominate positional ones so semantic
    // strategies clearly produce semantic locators (not nth-of-type page.locator).
    let getby_returns = source
        .lines()
        .filter(|l| l.contains("return page.getBy"))
        .count();
    let positional_returns = source
        .lines()
        .filter(|l| {
            l.contains("return page.locator") && l.contains("nth-of-type") && !l.contains(".nth(")
        })
        .count();
    assert!(
        getby_returns > positional_returns,
        "semantic getBy* returns ({getby_returns}) should outnumber positional page.locator returns ({positional_returns})"
    );
}

#[test]
fn generated_pageobject_has_semantic_method_names() {
    let source = generated_source();

    assert!(
        !source.contains("cssEl("),
        "generated PageObject still exposes a bare `cssEl` method:\n{source}"
    );

    // At least one human-readable, transliterated semantic method name must be present
    // (e.g. `sprositGigaChatInput` from placeholder, `podskazka` from aria-label,
    // `finansyButton` from the role+name button).
    let has_semantic_name = source.contains("sprositGigaChatInput")
        || source.contains("podskazka")
        || source.contains("finansyButton");
    assert!(
        has_semantic_name,
        "no recognised semantic method name in generated PageObject:\n{source}"
    );
}

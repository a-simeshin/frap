//! Frap Core — single public entry point for signature, clustering, and healing.
//!
//! Language SDKs (TypeScript WASM, future FFI/JSON-RPC) should depend on this crate,
//! not on the algorithm crates directly.

mod element_map;
mod error;
mod page_object;

pub use element_map::{
    build_element_map, build_element_map_json, filter_element_map, filter_element_map_json,
    recommend_locator, Cluster, ClusterType, ElementMap, ElementNode, FilterSpec,
    LocatorRecommendation, MapMetadata, MapOptions,
};
pub use page_object::{
    generate_page_object, generate_page_object_json, GenerateOptions, GeneratedArtifact,
    GeneratedFile,
};

#[cfg(feature = "wasm")]
pub mod wasm;

#[cfg(feature = "ffi")]
pub mod ffi;

pub use error::CoreError;

// Algorithm crates (also available as `frap_core::signature`, etc.)
pub use clustering;
pub use frap_rca::{
    analyze_timeline_json, PrimaryCause as RcaPrimaryCause, RcaReport,
    DEFAULT_WINDOW_MS as RCA_DEFAULT_WINDOW_MS,
};
pub use healing::{DOMElementInfo, DOMSnapshot, HealingEngine, HealingOrchestrator};
pub use signature::{
    calculate_attribute_bonus, calculate_confidence, calculate_identifier_conflict_penalty,
    calculate_identifier_migration_bonus, calculate_path_similarity,
    calculate_structural_similarity, calculate_token_similarity, extract_stable_attrs,
    levenshtein_distance, longest_common_subsequence_len, looks_like_generated,
    looks_like_semantic, Candidate, DOMToken, HealResult, Signature, DEFAULT_MIN_CONFIDENCE,
};

use serde::{Deserialize, Serialize};

/// JSON heal request (stable contract for WASM / JSON-RPC in later phases).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealRequest {
    pub primary_selector: String,
    pub original_signature: Signature,
    pub dom_snapshot: DOMSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f64>,
}

/// Platform API: orchestrates [`HealingEngine`] with optional JSON boundary.
#[derive(Debug, Clone)]
pub struct FrapCore {
    engine: HealingEngine,
}

impl FrapCore {
    pub fn new() -> Self {
        Self {
            engine: HealingEngine::new(),
        }
    }

    pub fn with_min_confidence(mut self, min_confidence: f64) -> Self {
        self.engine = self.engine.with_min_confidence(min_confidence);
        self
    }

    pub fn min_confidence(&self) -> f64 {
        self.engine.min_confidence
    }

    pub fn heal(
        &mut self,
        primary_selector: &str,
        original_signature: &Signature,
        dom_snapshot: &DOMSnapshot,
    ) -> HealResult {
        self.engine
            .heal(primary_selector, original_signature, dom_snapshot)
    }

    pub fn heal_request(&mut self, request: &HealRequest) -> HealResult {
        if let Some(min_confidence) = request.min_confidence {
            self.engine.min_confidence = min_confidence;
        }
        self.heal(
            &request.primary_selector,
            &request.original_signature,
            &request.dom_snapshot,
        )
    }

    /// Parse JSON request, run heal, return JSON [`HealResult`].
    pub fn heal_json(&mut self, input: &str) -> Result<String, CoreError> {
        let request: HealRequest = serde_json::from_str(input)?;
        let result = self.heal_request(&request);
        Ok(serde_json::to_string(&result)?)
    }

    /// Access underlying engine (e.g. for tests or advanced orchestration).
    pub fn engine_mut(&mut self) -> &mut HealingEngine {
        &mut self.engine
    }
}

/// Standalone RCA JSON API (no engine state required).
pub fn analyze_rca_json(
    timeline_json: &str,
    failure_at_ms: i64,
    window_ms: i64,
) -> Result<String, CoreError> {
    Ok(analyze_timeline_json(
        timeline_json,
        failure_at_ms,
        window_ms,
    )?)
}

impl Default for FrapCore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pay_button_signature() -> Signature {
        Signature {
            path: vec![DOMToken {
                tag: "button".to_string(),
                role: Some("submit".to_string()),
                semantic_type: None,
                structural_class: None,
                depth: 0,
            }],
            prefix: "button:submit".to_string(),
            stable_attrs: [("data-testid".to_string(), "pay-btn".to_string())].into(),
            text_content: Some("Pay".to_string()),
            position_in_parent: None,
            children_hash: 0,
            depth: 1,
        }
    }

    /// CP002-style: primary missing, candidate with new testid heals.
    fn cp002_snapshot() -> DOMSnapshot {
        DOMSnapshot {
            html: "<button data-testid='checkout-pay'>Pay</button>".to_string(),
            elements: vec![DOMElementInfo {
                selector: "[data-testid='checkout-pay']".to_string(),
                tag: "button".to_string(),
                attributes: [("data-testid".to_string(), "checkout-pay".to_string())].into(),
                text_content: Some("Pay".to_string()),
                path: vec!["button:submit".to_string()],
                position_in_parent: None,
                visible: None,
                computed_role: None,
                accessible_name: None,
                scope_hint: None,
            }],
        }
    }

    #[test]
    fn frap_core_heals_when_primary_missing() {
        let mut core = FrapCore::new();
        let result = core.heal(
            "[data-testid='pay-btn']",
            &pay_button_signature(),
            &cp002_snapshot(),
        );

        assert!(result.healed);
        assert_eq!(result.selector, "[data-testid='checkout-pay']");
        assert!(result.confidence >= DEFAULT_MIN_CONFIDENCE);
        assert!(!result.top_candidates.is_empty());
    }

    #[test]
    fn frap_core_primary_found_no_healing() {
        let mut core = FrapCore::new();
        let snapshot = cp002_snapshot();
        let selector = "[data-testid='checkout-pay']";
        let sig = pay_button_signature();

        let result = core.heal(selector, &sig, &snapshot);

        assert!(!result.healed);
        assert_eq!(result.selector, selector);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn schedule_heal_json_does_not_panic() {
        let request = HealRequest {
            primary_selector: "[data-testid='talk-open-healing']".to_string(),
            original_signature: Signature {
                path: vec![
                    DOMToken {
                        tag: "main".to_string(),
                        role: None,
                        semantic_type: None,
                        structural_class: None,
                        depth: 0,
                    },
                    DOMToken {
                        tag: "a".to_string(),
                        role: None,
                        semantic_type: None,
                        structural_class: None,
                        depth: 2,
                    },
                ],
                prefix: "main:->a:-".to_string(),
                stable_attrs: [("data-testid".to_string(), "talk-open-healing".to_string())].into(),
                text_content: Some("Смотреть доклад".to_string()),
                position_in_parent: None,
                children_hash: 0,
                depth: 3,
            },
            dom_snapshot: DOMSnapshot {
                html: String::new(),
                elements: vec![DOMElementInfo {
                    selector: "[data-testid='talk-card-open-healing']".to_string(),
                    tag: "a".to_string(),
                    attributes: [(
                        "data-testid".to_string(),
                        "talk-card-open-healing".to_string(),
                    )]
                    .into(),
                    text_content: Some("Смотреть доклад".to_string()),
                    path: vec![
                        "body:-".to_string(),
                        "main:-".to_string(),
                        "div:-".to_string(),
                        "a:-".to_string(),
                    ],
                    position_in_parent: None,
                    visible: None,
                    computed_role: None,
                    accessible_name: None,
                    scope_hint: None,
                }],
            },
            min_confidence: Some(0.7),
        };

        let json = serde_json::to_string(&request).expect("serialize");
        let mut core = FrapCore::new();
        let out = core.heal_json(&json).expect("heal_json");
        let result: HealResult = serde_json::from_str(&out).expect("parse");
        assert!(result.healed, "expected heal: {:?}", result);
    }

    #[test]
    fn heal_request_json_roundtrip() {
        let request = HealRequest {
            primary_selector: "[data-testid='pay-btn']".to_string(),
            original_signature: pay_button_signature(),
            dom_snapshot: cp002_snapshot(),
            min_confidence: Some(0.85),
        };

        let json = serde_json::to_string(&request).expect("serialize request");
        let parsed: HealRequest = serde_json::from_str(&json).expect("parse request");
        assert_eq!(parsed.primary_selector, request.primary_selector);
        assert_eq!(parsed.min_confidence, request.min_confidence);

        let mut core = FrapCore::new();
        let out = core.heal_json(&json).expect("heal_json");
        let result: HealResult = serde_json::from_str(&out).expect("parse result");
        assert!(result.healed);
    }

    #[test]
    fn reexports_match_subcrates() {
        let mut engine = HealingEngine::new();
        let mut core = FrapCore::new();
        let sig = pay_button_signature();
        let snap = cp002_snapshot();

        let from_engine = engine.heal("[data-testid='pay-btn']", &sig, &snap);
        let from_core = core.heal("[data-testid='pay-btn']", &sig, &snap);

        assert_eq!(from_engine.healed, from_core.healed);
        assert_eq!(from_engine.selector, from_core.selector);
    }

    #[test]
    fn heal_request_respects_min_confidence() {
        let mut core = FrapCore::new().with_min_confidence(0.99);
        let request = HealRequest {
            primary_selector: "[data-testid='pay-btn']".to_string(),
            original_signature: pay_button_signature(),
            dom_snapshot: cp002_snapshot(),
            min_confidence: Some(0.5),
        };

        let result = core.heal_request(&request);
        assert!(result.healed);
        assert_eq!(core.min_confidence(), 0.5);
    }

    /// CP003-style: two similar buttons — engine returns ranked candidates (may heal best match).
    #[test]
    fn ambiguous_snapshot_returns_candidates() {
        let mut core = FrapCore::new();
        let original = pay_button_signature();
        let snapshot = DOMSnapshot {
            html: String::new(),
            elements: vec![
                DOMElementInfo {
                    selector: "[data-testid='pay-a']".to_string(),
                    tag: "button".to_string(),
                    attributes: [("data-testid".to_string(), "pay-a".to_string())].into(),
                    text_content: Some("Pay".to_string()),
                    path: vec!["button:submit".to_string()],
                    position_in_parent: None,
                    visible: None,
                    computed_role: None,
                    accessible_name: None,
                    scope_hint: None,
                },
                DOMElementInfo {
                    selector: "[data-testid='pay-b']".to_string(),
                    tag: "button".to_string(),
                    attributes: [("data-testid".to_string(), "pay-b".to_string())].into(),
                    text_content: Some("Pay".to_string()),
                    path: vec!["button:submit".to_string()],
                    position_in_parent: None,
                    visible: None,
                    computed_role: None,
                    accessible_name: None,
                    scope_hint: None,
                },
            ],
        };

        let result = core.heal("[data-testid='pay-btn']", &original, &snapshot);
        assert!(!result.top_candidates.is_empty());
        assert!(result.top_candidates.len() <= 3);
        if result.healed {
            assert!(result.confidence >= DEFAULT_MIN_CONFIDENCE);
        }
    }

    #[test]
    fn clustering_reexport_usable() {
        let mut clusterer = clustering::DOMElementClusterer::new();
        let sig = pay_button_signature();
        clusterer.add_element("[data-testid='pay-btn']".to_string(), sig.clone());
        let found = clusterer.find_elements_in_cluster(&sig);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn signature_confidence_reexport() {
        let a = pay_button_signature();
        let b = a.clone();
        let c = calculate_confidence(&a, &b);
        assert!(c >= 0.9);
    }
}

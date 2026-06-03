use clustering::DOMElementClusterer;
use serde::{Deserialize, Serialize};
use signature::{calculate_confidence, Candidate, HealResult, Signature, DEFAULT_MIN_CONFIDENCE};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DOMSnapshot {
    pub html: String,
    pub elements: Vec<DOMElementInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DOMElementInfo {
    pub selector: String,
    pub tag: String,
    pub attributes: HashMap<String, String>,
    pub text_content: Option<String>,
    pub path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_in_parent: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computed_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessible_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealingEngine {
    pub min_confidence: f64,
    pub clusterer: DOMElementClusterer,
}

impl HealingEngine {
    pub fn new() -> Self {
        Self {
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            clusterer: DOMElementClusterer::new(),
        }
    }

    pub fn with_min_confidence(mut self, confidence: f64) -> Self {
        self.min_confidence = confidence;
        self
    }

    pub fn heal(
        &mut self,
        primary_selector: &str,
        original_signature: &Signature,
        dom_snapshot: &DOMSnapshot,
    ) -> HealResult {
        if let Some(element) = self.find_by_selector(primary_selector, dom_snapshot) {
            return HealResult::success(element.selector.clone());
        }

        let candidates = self.find_candidates(original_signature, dom_snapshot, true);

        if candidates.is_empty() {
            return HealResult::failed(vec![], original_signature.clone());
        }

        let best = match candidates.iter().max_by(|a, b| {
            a.confidence
                .partial_cmp(&b.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Some(c) => c.clone(),
            None => return HealResult::failed(vec![], original_signature.clone()),
        };

        if let Some(second) = candidates.get(1) {
            if best.confidence - second.confidence < 0.1 {
                return HealResult::failed(candidates, original_signature.clone());
            }
        }

        if best.confidence >= self.min_confidence {
            HealResult::healed(
                best.selector.clone(),
                best.confidence,
                candidates,
                original_signature.clone(),
            )
        } else {
            HealResult::failed(candidates, original_signature.clone())
        }
    }

    fn find_by_selector<'a>(
        &self,
        selector: &str,
        snapshot: &'a DOMSnapshot,
    ) -> Option<&'a DOMElementInfo> {
        snapshot.elements.iter().find(|e| e.selector == selector)
    }

    fn find_candidates(
        &mut self,
        original: &Signature,
        snapshot: &DOMSnapshot,
        cluster_only: bool,
    ) -> Vec<Candidate> {
        self.rebuild_clusterer(snapshot);

        let cluster_selectors: std::collections::HashSet<String> = self
            .clusterer
            .find_elements_in_cluster(original)
            .into_iter()
            .map(|e| e.selector.clone())
            .collect();

        let restrict = cluster_only && !cluster_selectors.is_empty();
        let mut candidates = Vec::new();

        for element_info in &snapshot.elements {
            if restrict && !cluster_selectors.contains(&element_info.selector) {
                continue;
            }

            let candidate_sig = Self::signature_from_element(element_info);
            let confidence = calculate_confidence(original, &candidate_sig);

            if confidence >= 0.5 {
                candidates.push(Candidate {
                    selector: element_info.selector.clone(),
                    signature: candidate_sig,
                    confidence,
                });
            }
        }

        if candidates.is_empty() && restrict {
            return self.find_candidates(original, snapshot, false);
        }

        candidates.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates
    }

    fn rebuild_clusterer(&mut self, snapshot: &DOMSnapshot) {
        self.clusterer = DOMElementClusterer::new();
        for element_info in &snapshot.elements {
            let sig = Self::signature_from_element(element_info);
            self.clusterer
                .add_element(element_info.selector.clone(), sig);
        }
    }

    /// Extract a structural signature from a captured DOM element (shared with element-map discovery).
    pub fn signature_from_element(element: &DOMElementInfo) -> Signature {
        Self::build_signature_from_element(element)
    }

    fn build_signature_from_element(element: &DOMElementInfo) -> Signature {
        use signature::{extract_stable_attrs, DOMToken};

        let mut tokens = Vec::new();
        for (i, path_token) in element.path.iter().enumerate() {
            let parts: Vec<&str> = path_token.split(':').collect();
            let tag = parts.first().copied().unwrap_or("unknown").to_string();
            let role = parts.get(1).filter(|&&r| r != "-").map(|&s| s.to_string());

            tokens.push(DOMToken {
                tag,
                role,
                semantic_type: None,
                structural_class: None,
                depth: i as u8,
            });
        }

        let prefix = tokens
            .iter()
            .take(5)
            .map(|t| format!("{}:{}", t.tag, t.role.as_deref().unwrap_or("-")))
            .collect::<Vec<_>>()
            .join(">");

        let attr_pairs: Vec<(String, String)> = element
            .attributes
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut stable_attrs = extract_stable_attrs(&attr_pairs);
        if let Some(testid) = element.attributes.get("data-testid") {
            stable_attrs.insert("data-testid".to_string(), testid.clone());
        }
        if let Some(id) = element.attributes.get("id") {
            stable_attrs.insert("id".to_string(), id.clone());
        }
        if let Some(data_id) = element.attributes.get("data-id") {
            stable_attrs.insert("data-id".to_string(), data_id.clone());
        }

        Signature {
            path: tokens,
            prefix,
            stable_attrs,
            text_content: element.text_content.clone(),
            position_in_parent: element.position_in_parent,
            children_hash: 0,
            depth: element.path.len() as u8,
        }
    }
}

impl Default for HealingEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealingOrchestrator {
    pub engine: HealingEngine,
    pub original_signatures: HashMap<String, Signature>,
}

impl HealingOrchestrator {
    pub fn new() -> Self {
        Self {
            engine: HealingEngine::new(),
            original_signatures: HashMap::new(),
        }
    }

    pub fn record_signature(&mut self, selector: String, signature: Signature) {
        self.original_signatures.insert(selector, signature);
    }

    pub fn heal_with_fallback(
        &mut self,
        primary_selector: &str,
        dom_snapshot: &DOMSnapshot,
    ) -> HealResult {
        let original = self.original_signatures.get(primary_selector);

        if let Some(original_sig) = original {
            self.engine
                .heal(primary_selector, original_sig, dom_snapshot)
        } else {
            HealResult::failed(
                vec![],
                Signature {
                    path: vec![],
                    prefix: String::new(),
                    stable_attrs: HashMap::new(),
                    text_content: None,
                    position_in_parent: None,
                    children_hash: 0,
                    depth: 0,
                },
            )
        }
    }
}

impl Default for HealingOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use signature::{DOMToken, Signature};

    #[test]
    fn dom_element_info_a11y_fields_default_when_absent() {
        // Old snapshot JSON without the a11y fields must still deserialize, with
        // all four new fields defaulting to `None` (backward compatibility).
        let without = r##"{
            "selector": "#pay",
            "tag": "button",
            "attributes": {},
            "text_content": "Pay",
            "path": ["button:-"]
        }"##;
        let el: DOMElementInfo = serde_json::from_str(without).expect("deserialize legacy");
        assert_eq!(el.visible, None);
        assert_eq!(el.computed_role, None);
        assert_eq!(el.accessible_name, None);
        assert_eq!(el.scope_hint, None);

        // A snapshot carrying the new fields deserializes them into `Some(..)`.
        let with = r##"{
            "selector": "#pay",
            "tag": "button",
            "attributes": {},
            "text_content": "Pay",
            "path": ["button:-"],
            "visible": true,
            "computed_role": "button",
            "accessible_name": "Pay",
            "scope_hint": "form#checkout"
        }"##;
        let el: DOMElementInfo = serde_json::from_str(with).expect("deserialize a11y");
        assert_eq!(el.visible, Some(true));
        assert_eq!(el.computed_role.as_deref(), Some("button"));
        assert_eq!(el.accessible_name.as_deref(), Some("Pay"));
        assert_eq!(el.scope_hint.as_deref(), Some("form#checkout"));
    }

    fn create_test_signature() -> Signature {
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

    fn create_test_snapshot() -> DOMSnapshot {
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
    fn test_healing_engine_new() {
        let engine = HealingEngine::new();
        assert_eq!(engine.min_confidence, DEFAULT_MIN_CONFIDENCE);
    }

    #[test]
    fn test_heal_success_when_element_found() {
        let mut engine = HealingEngine::new();
        let snapshot = DOMSnapshot {
            html: "<button>Pay</button>".to_string(),
            elements: vec![DOMElementInfo {
                selector: "#pay-btn".to_string(),
                tag: "button".to_string(),
                attributes: HashMap::new(),
                text_content: Some("Pay".to_string()),
                path: vec!["button:-".to_string()],
                position_in_parent: None,
                visible: None,
                computed_role: None,
                accessible_name: None,
                scope_hint: None,
            }],
        };

        let original = create_test_signature();
        let result = engine.heal("#pay-btn", &original, &snapshot);

        assert!(!result.healed);
        assert_eq!(result.confidence, 1.0);
        assert_eq!(result.selector, "#pay-btn");
    }

    #[test]
    fn test_healing_orchestrator() {
        let mut orchestrator = HealingOrchestrator::new();
        let sig = create_test_signature();

        orchestrator.record_signature("[data-testid='pay-btn']".to_string(), sig);

        assert_eq!(orchestrator.original_signatures.len(), 1);
    }

    #[test]
    fn test_heal_with_fallback_no_original() {
        let mut orchestrator = HealingOrchestrator::new();
        let snapshot = create_test_snapshot();

        let result = orchestrator.heal_with_fallback("[data-testid='unknown']", &snapshot);

        assert!(!result.healed);
        assert_eq!(result.confidence, 0.0);
    }
}

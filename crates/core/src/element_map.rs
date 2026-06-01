//! Element map discovery: cluster DOM snapshot elements for Page Object generation.

use crate::CoreError;
use clustering::DOMElementClusterer;
use healing::{DOMElementInfo, DOMSnapshot, HealingEngine};
use serde::{Deserialize, Serialize};
use signature::Signature;
use std::collections::{HashMap, HashSet};

const INTERACTIVE_TAGS: &[&str] = &["button", "a", "input", "select", "textarea"];

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
    /// Reliability of the recommended **selector** — how trustworthy the chosen
    /// locator is, driven by `strategy` (see [`selector_stability_score`]).
    /// Distinct from [`ElementNode::confidence`]; both rank a `data-testid`
    /// highest but live on different scales.
    pub confidence: f64,
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
    /// Structural confidence of the element **signature** (used by healing and
    /// clustering), from [`stability_confidence`]. This is NOT the selector
    /// reliability — that is [`LocatorRecommendation::confidence`]. Both are
    /// monotonic by quality (`data-testid` ≥ `id` ≥ `data-id` ≥ text ≥ none).
    pub confidence: f64,
    pub locator: LocatorRecommendation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClusterType {
    Single,
    List,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub id: String,
    pub cluster_type: ClusterType,
    pub element_ids: Vec<String>,
    pub prefix_signature: String,
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

pub fn recommend_locator(element: &DOMElementInfo) -> LocatorRecommendation {
    let selector = best_selector(element);
    let confidence = selector_stability_score(element);
    let strategy = if element.attributes.contains_key("data-testid") {
        "data-testid"
    } else if element.attributes.contains_key("id") {
        "id"
    } else if element.attributes.contains_key("data-id") {
        "data-id"
    } else if element.attributes.contains_key("name") {
        "name"
    } else {
        "css"
    }
    .to_string();

    LocatorRecommendation {
        selector: selector.clone(),
        strategy,
        confidence,
    }
}

pub fn build_element_map(snapshot: &DOMSnapshot, options: &MapOptions) -> ElementMap {
    let mut clusterer = DOMElementClusterer::new();
    let mut cluster_members: HashMap<String, Vec<String>> = HashMap::new();
    let mut cluster_prefix: HashMap<String, String> = HashMap::new();
    let mut elements = Vec::new();

    let limit = options.max_elements.unwrap_or(snapshot.elements.len());

    for (idx, element) in snapshot.elements.iter().take(limit).enumerate() {
        if !options.include_non_interactive && !is_interactive(element) {
            continue;
        }

        let signature = HealingEngine::signature_from_element(element);
        let cluster_id = clusterer.add_element(element.selector.clone(), signature.clone());
        let element_id = format!("el-{idx}");
        cluster_prefix
            .entry(cluster_id.clone())
            .or_insert_with(|| signature.prefix.clone());
        cluster_members
            .entry(cluster_id.clone())
            .or_default()
            .push(element_id.clone());

        let locator = recommend_locator(element);
        let confidence = stability_confidence(&signature);

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

    let clusters = cluster_members
        .into_iter()
        .map(|(id, element_ids)| {
            let cluster_type = if element_ids.len() >= 2 {
                ClusterType::List
            } else {
                ClusterType::Single
            };
            Cluster {
                id: id.clone(),
                cluster_type,
                element_ids,
                prefix_signature: cluster_prefix.get(&id).cloned().unwrap_or_default(),
            }
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
                Some(Cluster {
                    id: c.id.clone(),
                    cluster_type: if element_ids.len() >= 2 {
                        ClusterType::List
                    } else {
                        ClusterType::Single
                    },
                    element_ids,
                    prefix_signature: c.prefix_signature.clone(),
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

fn best_selector(element: &DOMElementInfo) -> String {
    if let Some(testid) = element.attributes.get("data-testid") {
        return format!("[data-testid=\"{testid}\"]");
    }
    if let Some(id) = element.attributes.get("id") {
        if !looks_generated_id(id) {
            return format!("#{}", escape_css_attr(id));
        }
    }
    if let Some(data_id) = element.attributes.get("data-id") {
        return format!("[data-id=\"{data_id}\"]");
    }
    if let Some(name) = element.attributes.get("name") {
        return format!("[name=\"{name}\"]");
    }
    element.selector.clone()
}

fn looks_generated_id(id: &str) -> bool {
    signature::looks_like_generated(id)
}

fn escape_css_attr(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Reliability of the recommended **selector**, by the strongest attribute.
/// Monotonic: `data-testid` > `id` > `data-id` > none.
fn selector_stability_score(element: &DOMElementInfo) -> f64 {
    if element.attributes.contains_key("data-testid") {
        0.95
    } else if element.attributes.contains_key("id") {
        0.85
    } else if element.attributes.contains_key("data-id") {
        0.8
    } else {
        0.5
    }
}

/// Structural confidence of the element **signature** (for healing/clustering).
/// Monotonic by quality so it never inverts against [`selector_stability_score`]:
/// `data-testid` > `id` > `data-id` > text-only > none.
fn stability_confidence(signature: &Signature) -> f64 {
    let mut score: f64 = 0.5;
    if signature.stable_attrs.contains_key("data-testid") {
        score += 0.3;
    }
    if signature.stable_attrs.contains_key("id") {
        score += 0.15;
    }
    // Previously omitted, which made a data-id element tie with a no-signal one.
    if signature.stable_attrs.contains_key("data-id") {
        score += 0.1;
    }
    if signature
        .text_content
        .as_ref()
        .is_some_and(|t| !t.is_empty())
    {
        score += 0.05;
    }
    score.min(1.0)
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
                },
                DOMElementInfo {
                    selector: "[data-testid=\"card-2\"]".to_string(),
                    tag: "article".to_string(),
                    attributes: [("data-testid".to_string(), "card-2".to_string())].into(),
                    text_content: Some("Card 2".to_string()),
                    path: vec!["div:-".to_string(), "article:-".to_string()],
                    position_in_parent: Some(1),
                },
                DOMElementInfo {
                    selector: "button.buy".to_string(),
                    tag: "button".to_string(),
                    attributes: HashMap::new(),
                    text_content: Some("Buy".to_string()),
                    path: vec!["button:-".to_string()],
                    position_in_parent: None,
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

    // issue #09: the node confidence and the locator confidence are two distinct
    // scales, but neither may invert against element quality, and a data-id must
    // not tie with a no-signal element.
    #[test]
    fn confidence_scales_are_monotonic_and_consistent() {
        let mk = |attrs: &[(&str, &str)], text: Option<&str>| {
            let snap = DOMSnapshot {
                html: String::new(),
                elements: vec![DOMElementInfo {
                    selector: "x".into(),
                    tag: "button".into(),
                    attributes: attrs
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                    text_content: text.map(|t| t.to_string()),
                    path: vec!["button:-".into()],
                    position_in_parent: None,
                }],
            };
            let m = build_element_map(&snap, &MapOptions::default());
            let e = &m.elements[0];
            (e.confidence, e.locator.confidence)
        };

        let testid = mk(&[("data-testid", "x")], None);
        let id = mk(&[("id", "x")], None);
        let data_id = mk(&[("data-id", "x")], None);
        let text = mk(&[], Some("Buy"));
        let none = mk(&[], None);

        // node (structural) scale strictly decreasing by quality
        assert!(testid.0 > id.0 && id.0 > data_id.0 && data_id.0 > text.0 && text.0 > none.0);
        // the bug this fixes: data-id must beat a no-signal element
        assert!(data_id.0 > none.0);
        // locator scale monotonic (non-strict)
        assert!(testid.1 >= id.1 && id.1 >= data_id.1 && data_id.1 >= none.1);
        // no inversion: both scales rank testid >= id >= data-id
        assert!(testid.0 >= id.0 && testid.1 >= id.1);
        assert!(id.0 >= data_id.0 && id.1 >= data_id.1);
    }
}

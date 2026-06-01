use serde::{Deserialize, Serialize};
use signature::Signature;
use std::collections::HashMap;

pub const MAX_DEPTH: usize = 5;
pub const SIMILARITY_THRESHOLD: f64 = 0.7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusteredElement {
    pub selector: String,
    pub signature: Signature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNode {
    pub id: String,
    pub prefix_signature: String,
    pub elements: Vec<ClusteredElement>,
    pub signature_template: Signature,
}

impl ClusterNode {
    pub fn new(prefix: String) -> Self {
        let id = format!("cluster_{}", prefix.replace(">", "_").replace(":", "_"));
        Self {
            id,
            prefix_signature: prefix.clone(),
            elements: vec![],
            signature_template: Signature {
                path: vec![],
                prefix,
                stable_attrs: HashMap::new(),
                text_content: None,
                position_in_parent: None,
                children_hash: 0,
                depth: 0,
            },
        }
    }

    pub fn add_element(&mut self, selector: String, signature: Signature) {
        self.elements.push(ClusteredElement {
            selector,
            signature,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn update_template(&mut self, signature: &Signature) {
        if self.elements.len() == 1 {
            self.signature_template = signature.clone();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseTreeNode {
    pub token_value: String,
    pub depth: u8,
    pub children: HashMap<String, ParseTreeNode>,
    pub clusters: Vec<ClusterNode>,
}

impl ParseTreeNode {
    pub fn new(token_value: String, depth: u8) -> Self {
        Self {
            token_value,
            depth,
            children: HashMap::new(),
            clusters: vec![],
        }
    }

    pub fn has_child(&self, key: &str) -> bool {
        self.children.contains_key(key)
    }

    pub fn add_child(&mut self, key: String, node: ParseTreeNode) {
        self.children.insert(key, node);
    }

    pub fn get_child_mut(&mut self, key: &str) -> Option<&mut ParseTreeNode> {
        self.children.get_mut(key)
    }

    pub fn add_cluster(&mut self, cluster: ClusterNode) {
        self.clusters.push(cluster);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseTree {
    pub root: HashMap<String, ParseTreeNode>,
}

impl ParseTree {
    pub fn new() -> Self {
        Self {
            root: HashMap::new(),
        }
    }

    pub fn find_or_create_cluster(&mut self, signature: &Signature) -> &mut ClusterNode {
        let token_count = signature.path.len().min(MAX_DEPTH);
        if token_count == 0 {
            panic!("Could not find or create cluster: empty signature path");
        }

        let node = self.find_or_create_leaf_node(signature, token_count);

        let best_cluster = Self::find_best_cluster(node, signature);

        if let Some(idx) = best_cluster {
            return &mut node.clusters[idx];
        }

        // The cluster id is derived from the prefix, which can be shared by
        // different leaf tags. Append the leaf tag so a split here stays split
        // when callers group by `cluster.id`.
        let mut new_cluster = ClusterNode::new(signature.prefix.clone());
        if let Some(leaf) = signature.path.last() {
            new_cluster.id = format!("{}__{}", new_cluster.id, leaf.tag);
        }
        node.add_cluster(new_cluster);
        let idx = node.clusters.len() - 1;
        &mut node.clusters[idx]
    }

    fn find_or_create_leaf_node(
        &mut self,
        signature: &Signature,
        token_count: usize,
    ) -> &mut ParseTreeNode {
        let mut current_map = &mut self.root;

        for i in 0..token_count {
            let token = &signature.path[i];
            let key = format!("{}:{}", token.tag, token.role.as_deref().unwrap_or("-"));

            if !current_map.contains_key(&key) {
                current_map.insert(key.clone(), ParseTreeNode::new(key.clone(), i as u8));
            }

            let node = current_map.get_mut(&key).unwrap();

            if i == token_count - 1 {
                return node;
            }

            if i + 1 >= signature.path.len() {
                break;
            }

            let next_token = &signature.path[i + 1];
            let next_key = format!(
                "{}:{}",
                next_token.tag,
                next_token.role.as_deref().unwrap_or("-")
            );

            if !node.has_child(&next_key) {
                node.add_child(
                    next_key.clone(),
                    ParseTreeNode::new(next_key.clone(), (i + 1) as u8),
                );
            }

            current_map = &mut node.get_child_mut(&next_key).unwrap().children;
        }

        panic!("Could not find or create cluster: invalid path traversal");
    }

    fn find_best_cluster(node: &ParseTreeNode, signature: &Signature) -> Option<usize> {
        let mut best_score = 0.0;
        let mut best_idx = None;

        for (idx, cluster) in node.clusters.iter().enumerate() {
            // Keep different element types apart (e.g. <a> vs <button> that share
            // the same top path prefix) so a LIST stays a homogeneous component.
            if !same_leaf_tag(&cluster.signature_template, signature) {
                continue;
            }
            let score = Self::calculate_cluster_similarity(&cluster.signature_template, signature);
            if score > best_score && score >= SIMILARITY_THRESHOLD {
                best_score = score;
                best_idx = Some(idx);
            }
        }

        best_idx
    }

    fn calculate_cluster_similarity(template: &Signature, signature: &Signature) -> f64 {
        use signature::{
            calculate_path_similarity, calculate_structural_similarity, calculate_token_similarity,
        };

        let path_sim = calculate_path_similarity(&template.prefix, &signature.prefix);
        let token_sim = calculate_token_similarity(&template.path, &signature.path);
        let structural_sim =
            calculate_structural_similarity(template.children_hash, signature.children_hash);

        0.5 * path_sim + 0.3 * token_sim + 0.2 * structural_sim
    }

    pub fn find_cluster<'a>(&'a self, signature: &'a Signature) -> Option<&'a ClusterNode> {
        let mut current = &self.root;
        let token_count = signature.path.len().min(MAX_DEPTH);

        for i in 0..token_count {
            let token = &signature.path[i];
            let key = format!("{}:{}", token.tag, token.role.as_deref().unwrap_or("-"));

            if !current.contains_key(&key) {
                return None;
            }

            if i == token_count - 1 {
                let node = current.get(&key)?;
                return Self::find_best_cluster_readonly(node, signature);
            }

            let node = current.get(&key)?;
            if i + 1 < signature.path.len() {
                let _next_token = &signature.path[i + 1];
                current = &node.children;
            } else {
                return None;
            }
        }

        None
    }

    fn find_best_cluster_readonly<'a>(
        node: &'a ParseTreeNode,
        signature: &'a Signature,
    ) -> Option<&'a ClusterNode> {
        let mut best_score = 0.0;
        let mut best_cluster = None;

        for cluster in &node.clusters {
            if !same_leaf_tag(&cluster.signature_template, signature) {
                continue;
            }
            let score =
                Self::calculate_cluster_similarity_readonly(&cluster.signature_template, signature);
            if score > best_score && score >= SIMILARITY_THRESHOLD {
                best_score = score;
                best_cluster = Some(cluster);
            }
        }

        best_cluster
    }

    fn calculate_cluster_similarity_readonly(template: &Signature, signature: &Signature) -> f64 {
        use signature::{
            calculate_path_similarity, calculate_structural_similarity, calculate_token_similarity,
        };

        let path_sim = calculate_path_similarity(&template.prefix, &signature.prefix);
        let token_sim = calculate_token_similarity(&template.path, &signature.path);
        let structural_sim =
            calculate_structural_similarity(template.children_hash, signature.children_hash);

        0.5 * path_sim + 0.3 * token_sim + 0.2 * structural_sim
    }
}

impl Default for ParseTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Two signatures belong to the same component only if their leaf element has the
/// same tag. This stops a shared top prefix from merging different controls.
fn same_leaf_tag(a: &Signature, b: &Signature) -> bool {
    match (a.path.last(), b.path.last()) {
        (Some(ta), Some(tb)) => ta.tag == tb.tag,
        (None, None) => true,
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DOMElementClusterer {
    pub tree: ParseTree,
}

impl DOMElementClusterer {
    pub fn new() -> Self {
        Self {
            tree: ParseTree::new(),
        }
    }

    pub fn add_element(&mut self, selector: String, signature: Signature) -> String {
        let cluster = self.tree.find_or_create_cluster(&signature);
        let cluster_id = cluster.id.clone();
        cluster.add_element(selector.clone(), signature.clone());
        cluster.update_template(&signature);
        cluster_id
    }

    pub fn find_cluster<'a>(&'a self, signature: &'a Signature) -> Option<&'a ClusterNode> {
        self.tree.find_cluster(signature)
    }

    pub fn find_elements_in_cluster<'a>(
        &'a self,
        signature: &'a Signature,
    ) -> Vec<&'a ClusteredElement> {
        if let Some(cluster) = self.find_cluster(signature) {
            cluster.elements.iter().collect()
        } else {
            vec![]
        }
    }
}

impl Default for DOMElementClusterer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use signature::Signature;
    use std::collections::HashMap;

    fn create_test_signature(tag: &str, role: Option<&str>) -> Signature {
        Signature {
            path: vec![signature::DOMToken {
                tag: tag.to_string(),
                role: role.map(|s| s.to_string()),
                semantic_type: None,
                structural_class: None,
                depth: 0,
            }],
            prefix: format!("{}:{}", tag, role.unwrap_or("-")),
            stable_attrs: HashMap::new(),
            text_content: None,
            position_in_parent: None,
            children_hash: 0,
            depth: 1,
        }
    }

    #[test]
    fn test_clusterer_add_element() {
        let mut clusterer = DOMElementClusterer::new();
        let sig = create_test_signature("button", Some("submit"));
        let cluster_id = clusterer.add_element("[data-testid='pay-btn']".to_string(), sig.clone());

        assert!(!cluster_id.is_empty());

        let elements = clusterer.find_elements_in_cluster(&sig);
        assert_eq!(elements.len(), 1);
    }

    #[test]
    fn test_cluster_grouping() {
        let mut clusterer = DOMElementClusterer::new();

        let sig1 = create_test_signature("button", Some("submit"));
        let sig2 = create_test_signature("button", Some("submit"));

        clusterer.add_element("#btn1".to_string(), sig1.clone());
        clusterer.add_element("#btn2".to_string(), sig2.clone());

        let elements = clusterer.find_elements_in_cluster(&sig1);
        assert_eq!(elements.len(), 2);
    }

    #[test]
    fn test_parse_tree_new() {
        let tree = ParseTree::new();
        assert!(tree.root.is_empty());
    }

    #[test]
    fn test_cluster_node_new() {
        let cluster = ClusterNode::new("div:main>button:action".to_string());
        assert!(cluster.elements.is_empty());
        assert!(!cluster.id.is_empty());
    }

    fn tok(tag: &str) -> signature::DOMToken {
        signature::DOMToken {
            tag: tag.to_string(),
            role: None,
            semantic_type: None,
            structural_class: None,
            depth: 0,
        }
    }

    // issue #03: elements that share the top-5 path prefix but have a different
    // leaf tag (e.g. <a> vs <button> deep under `main`) must NOT merge into one
    // LIST. Same leaf tag must still group.
    #[test]
    fn different_leaf_tags_do_not_merge() {
        let deep = |leaf: &str| {
            let mut path = vec![tok("div"); 9];
            path.push(tok(leaf));
            Signature {
                path,
                prefix: "div:->div:->div:->div:->div:-".to_string(),
                stable_attrs: HashMap::new(),
                text_content: None,
                position_in_parent: None,
                children_hash: 0,
                depth: 10,
            }
        };

        let mut clusterer = DOMElementClusterer::new();
        let id_link = clusterer.add_element("link".to_string(), deep("a"));
        let id_btn = clusterer.add_element("button".to_string(), deep("button"));
        let id_link2 = clusterer.add_element("link2".to_string(), deep("a"));

        assert_ne!(id_link, id_btn, "different leaf tags must not share a cluster");
        assert_eq!(id_link, id_link2, "same leaf tag must group together");
    }
}

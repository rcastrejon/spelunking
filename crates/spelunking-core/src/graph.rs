use petgraph::{
    graph::{DiGraph, NodeIndex},
    visit::EdgeRef,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    SourceFile,
    App,
    Model,
    View,
    Url,
    Serializer,
    Form,
    Signal,
    Task,
    Middleware,
}

impl NodeType {
    fn as_id_prefix(self) -> &'static str {
        match self {
            Self::SourceFile => "source_file",
            Self::App => "app",
            Self::Model => "model",
            Self::View => "view",
            Self::Url => "url",
            Self::Serializer => "serializer",
            Self::Form => "form",
            Self::Signal => "signal",
            Self::Task => "task",
            Self::Middleware => "middleware",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    Contains,
    Inherits,
    RoutesTo,
    Queries,
    Serializes,
    Triggers,
    Intercepts,
    RelatesTo,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeKey {
    node_type: NodeType,
    identifier: String,
}

impl NodeKey {
    pub fn new(node_type: NodeType, identifier: impl Into<String>) -> Self {
        Self {
            node_type,
            identifier: identifier.into(),
        }
    }

    fn id(&self) -> String {
        format!("{}:{}", self.node_type.as_id_prefix(), self.identifier)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphExport {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl GraphExport {
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn node_count_by_type(&self, node_type: NodeType) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.node_type == node_type)
            .count()
    }

    pub fn edge_count_by_type(&self, edge_type: EdgeType) -> usize {
        self.edges
            .iter()
            .filter(|edge| edge.edge_type == edge_type)
            .count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EdgeData {
    edge_type: EdgeType,
}

#[derive(Debug, Default)]
pub struct GraphBuilder {
    graph: DiGraph<Node, EdgeData>,
    indices_by_key: HashMap<NodeKey, NodeIndex>,
    edge_keys: HashSet<(usize, usize, EdgeType)>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(
        &mut self,
        key: NodeKey,
        label: impl Into<String>,
        path: Option<String>,
    ) -> NodeIndex {
        if let Some(index) = self.indices_by_key.get(&key) {
            return *index;
        }

        let node = Node {
            id: key.id(),
            node_type: key.node_type,
            label: label.into(),
            path,
        };
        let index = self.graph.add_node(node);

        self.indices_by_key.insert(key, index);
        index
    }

    pub fn add_edge(&mut self, source: NodeIndex, target: NodeIndex, edge_type: EdgeType) {
        if self
            .edge_keys
            .insert((source.index(), target.index(), edge_type))
        {
            self.graph.add_edge(source, target, EdgeData { edge_type });
        }
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn export(&self) -> GraphExport {
        let nodes = self
            .graph
            .node_indices()
            .map(|index| self.graph[index].clone())
            .collect();
        let edges = self
            .graph
            .edge_references()
            .map(|edge| Edge {
                source: self.graph[edge.source()].id.clone(),
                target: self.graph[edge.target()].id.clone(),
                edge_type: edge.weight().edge_type,
            })
            .collect();

        GraphExport { nodes, edges }
    }
}

pub fn build_source_file_graph(root: impl AsRef<Path>, paths: &[PathBuf]) -> GraphExport {
    let root = root.as_ref();
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut builder = GraphBuilder::new();

    add_source_file_nodes(&mut builder, &root, paths);

    builder.export()
}

pub fn add_source_file_nodes(
    builder: &mut GraphBuilder,
    root: impl AsRef<Path>,
    paths: &[PathBuf],
) -> HashMap<PathBuf, NodeIndex> {
    let root = root.as_ref();
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut source_files_by_path = HashMap::new();

    for path in paths {
        let index = add_source_file_node(builder, &root, path);
        source_files_by_path.insert(canonical_path(path), index);
    }

    source_files_by_path
}

pub fn add_source_file_node(
    builder: &mut GraphBuilder,
    root: impl AsRef<Path>,
    path: impl AsRef<Path>,
) -> NodeIndex {
    let root = root.as_ref();
    let path = path.as_ref();
    let relative_path = relative_path_identifier(root, path);

    builder.add_node(
        NodeKey::new(NodeType::SourceFile, relative_path.clone()),
        relative_path.clone(),
        Some(relative_path),
    )
}

pub fn relative_path_identifier(root: impl AsRef<Path>, path: impl AsRef<Path>) -> String {
    let root = canonical_path(root.as_ref());
    let path = canonical_path(path.as_ref());
    let relative = path.strip_prefix(&root).unwrap_or(&path);

    normalize_path(relative)
}

pub fn canonical_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();

    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deduplicates_nodes_by_key() {
        let mut builder = GraphBuilder::new();
        let key = NodeKey::new(NodeType::SourceFile, "app/models.py");

        let first = builder.add_node(
            key.clone(),
            "app/models.py",
            Some("app/models.py".to_owned()),
        );
        let second = builder.add_node(key, "models.py", None);

        assert_eq!(first, second);
        assert_eq!(builder.node_count(), 1);

        let graph = builder.export();
        assert_eq!(graph.nodes[0].label, "app/models.py");
        assert_eq!(graph.nodes[0].path.as_deref(), Some("app/models.py"));
    }

    #[test]
    fn exports_typed_edges_with_node_ids() {
        let mut builder = GraphBuilder::new();
        let source = builder.add_node(NodeKey::new(NodeType::Url, "products/"), "products/", None);
        let target = builder.add_node(
            NodeKey::new(NodeType::View, "products.index"),
            "index",
            None,
        );

        builder.add_edge(source, target, EdgeType::RoutesTo);

        let graph = builder.export();

        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].source, "url:products/");
        assert_eq!(graph.edges[0].target, "view:products.index");
        assert_eq!(graph.edges[0].edge_type, EdgeType::RoutesTo);
    }

    #[test]
    fn deduplicates_edges_by_source_target_and_type() {
        let mut builder = GraphBuilder::new();
        let source = builder.add_node(NodeKey::new(NodeType::Url, "products/"), "products/", None);
        let target = builder.add_node(
            NodeKey::new(NodeType::View, "products.index"),
            "index",
            None,
        );

        builder.add_edge(source, target, EdgeType::RoutesTo);
        builder.add_edge(source, target, EdgeType::RoutesTo);

        assert_eq!(builder.edge_count(), 1);
    }

    #[test]
    fn builds_source_file_graph_with_relative_paths() {
        let root = std::env::temp_dir().join(format!("spelunking-graph-{}", std::process::id()));
        let file = root.join("app/models.py");

        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();

        let graph = build_source_file_graph(&root, std::slice::from_ref(&file));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].id, "source_file:app/models.py");
        assert_eq!(graph.nodes[0].node_type, NodeType::SourceFile);
        assert_eq!(graph.nodes[0].label, "app/models.py");
        assert_eq!(graph.nodes[0].path.as_deref(), Some("app/models.py"));
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn indexes_source_file_nodes_by_canonical_path() {
        let root =
            std::env::temp_dir().join(format!("spelunking-graph-index-{}", std::process::id()));
        let file = root.join("app/models.py");

        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();
        let expected_key = file.canonicalize().unwrap();

        let mut builder = GraphBuilder::new();
        let source_files = add_source_file_nodes(&mut builder, &root, std::slice::from_ref(&file));
        let _ = std::fs::remove_dir_all(&root);

        assert!(source_files.contains_key(&expected_key));
        assert_eq!(builder.node_count(), 1);
    }
}

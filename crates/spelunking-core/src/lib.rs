//! Core library for Spelunking.

pub mod discovery;
pub mod graph;
pub mod parsing;

pub use discovery::{DiscoveryError, discover_python_files};
pub use graph::{
    Edge, EdgeType, GraphBuilder, GraphExport, Node, NodeKey, NodeType, build_source_file_graph,
};
pub use parsing::{
    ParsedPythonModule, PythonParseDiagnostic, PythonParseDiagnosticKind, PythonParseReport,
    parse_python_files,
};

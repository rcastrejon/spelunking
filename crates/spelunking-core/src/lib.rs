//! Core library for Spelunking.

pub mod analysis;
pub mod discovery;
pub mod graph;
pub mod parsing;

pub use analysis::python::subject::{
    DjangoBehaviorPath, DjangoBehaviorReport, DjangoBehaviorStep, DjangoLifecycleCandidate,
    DjangoMutationSite, DjangoRelatedComponent, DjangoRelatedModel, DjangoRelevantMethod,
    DjangoSubjectCandidate, DjangoSubjectError, DjangoSubjectEvidence, DjangoSubjectField,
    DjangoSubjectModel, DjangoSubjectReport, DjangoSubjectState, inspect_django_behavior,
    inspect_django_subject,
};
pub use analysis::{
    AnalysisContext, AnalysisPipeline, Analyzer, SourceLanguage, analyze_python_project,
};
pub use discovery::{DiscoveryError, discover_python_files};
pub use graph::{
    Edge, EdgeType, GraphBuilder, GraphExport, GraphFilter, Node, NodeKey, NodeType,
    build_source_file_graph, relative_path_identifier,
};
pub use parsing::{
    ParsedPythonModule, PythonParseDiagnostic, PythonParseDiagnosticKind, PythonParseReport,
    parse_python_files,
};

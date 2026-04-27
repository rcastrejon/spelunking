//! Core library for Spelunking.

pub mod analysis;
pub mod discovery;
pub mod graph;
pub mod parsing;

pub use analysis::python::subject::{
    DJANGO_DOMAIN_FACT_BASES, DJANGO_DOMAIN_FACT_ORIGINS, DJANGO_DOMAIN_FACT_SCHEMA_VERSION,
    DJANGO_DOMAIN_FACT_STATUSES, DJANGO_DOMAIN_FACT_TYPES, DJANGO_DOMAIN_FLOW_SCHEMA_VERSION,
    DJANGO_EVIDENCE_PACK_SCHEMA_VERSION, DjangoArtifactBundle, DjangoBehaviorPath,
    DjangoBehaviorReport, DjangoBehaviorStep, DjangoCouplingSignal, DjangoDomainFact,
    DjangoDomainFlow, DjangoDomainFlowFinding, DjangoDomainFlowStep, DjangoEvidenceConfidence,
    DjangoEvidenceLifecycle, DjangoEvidencePack, DjangoEvidenceRelationshipMap,
    DjangoGuidanceBasis, DjangoGuidanceReport, DjangoGuidanceSubjectSlice,
    DjangoLifecycleCandidate, DjangoMutationSite, DjangoOpenQuestion, DjangoReadingPathEntry,
    DjangoRelatedComponent, DjangoRelatedModel, DjangoRelatedTest, DjangoRelevantMethod,
    DjangoRiskSignal, DjangoSubjectCandidate, DjangoSubjectError, DjangoSubjectEvidence,
    DjangoSubjectField, DjangoSubjectModel, DjangoSubjectReport, DjangoSubjectState,
    build_django_artifact_bundle, build_django_evidence_pack, django_subject_slug,
    extract_django_domain_facts, extract_django_domain_facts_from_packs, inspect_django_behavior,
    inspect_django_guidance, inspect_django_subject, interpret_django_domain_flows,
    interpret_django_domain_flows_from_packs, interpret_django_domain_flows_from_packs_and_facts,
    render_django_domain_facts_jsonl, render_django_domain_flow_markdown,
    render_django_evaluation_report, render_django_markdown_report,
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

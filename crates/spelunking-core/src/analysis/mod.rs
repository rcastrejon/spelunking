pub mod python;

use crate::{
    graph::{GraphBuilder, GraphExport, add_source_file_nodes, canonical_path},
    parsing::ParsedPythonModule,
};
use petgraph::graph::NodeIndex;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLanguage {
    Python,
}

pub trait Analyzer {
    fn name(&self) -> &'static str;
    fn language(&self) -> SourceLanguage;
    fn analyze(&self, context: &AnalysisContext<'_>, graph: &mut GraphBuilder);
}

pub struct AnalysisPipeline {
    analyzers: Vec<Box<dyn Analyzer>>,
}

impl AnalysisPipeline {
    pub fn new() -> Self {
        Self {
            analyzers: Vec::new(),
        }
    }

    pub fn python_django() -> Self {
        Self::new().with_analyzer(python::django::DjangoModelAnalyzer)
    }

    pub fn with_analyzer(mut self, analyzer: impl Analyzer + 'static) -> Self {
        self.analyzers.push(Box::new(analyzer));
        self
    }

    pub fn run(&self, context: &AnalysisContext<'_>, graph: &mut GraphBuilder) {
        for analyzer in &self.analyzers {
            analyzer.analyze(context, graph);
        }
    }
}

impl Default for AnalysisPipeline {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AnalysisContext<'a> {
    root: PathBuf,
    python_modules: &'a [ParsedPythonModule],
    source_files_by_path: HashMap<PathBuf, NodeIndex>,
}

impl<'a> AnalysisContext<'a> {
    pub fn new(
        root: impl AsRef<Path>,
        python_modules: &'a [ParsedPythonModule],
        source_files_by_path: HashMap<PathBuf, NodeIndex>,
    ) -> Self {
        Self {
            root: canonical_path(root.as_ref()),
            python_modules,
            source_files_by_path,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn python_modules(&self) -> &'a [ParsedPythonModule] {
        self.python_modules
    }

    pub fn source_file_index(&self, path: impl AsRef<Path>) -> Option<NodeIndex> {
        self.source_files_by_path
            .get(&canonical_path(path.as_ref()))
            .copied()
    }
}

pub fn analyze_python_project(
    root: impl AsRef<Path>,
    python_files: &[PathBuf],
    python_modules: &[ParsedPythonModule],
) -> GraphExport {
    let root = canonical_path(root.as_ref());
    let mut graph = GraphBuilder::new();
    let source_files_by_path = add_source_file_nodes(&mut graph, &root, python_files);
    let context = AnalysisContext::new(&root, python_modules, source_files_by_path);

    AnalysisPipeline::python_django().run(&context, &mut graph);

    graph.export()
}

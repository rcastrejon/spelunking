use clap::{Parser, ValueEnum};
use serde::Serialize;
use spelunking_core::{
    Edge, EdgeType, GraphExport, GraphFilter, Node, NodeType, PythonParseDiagnostic,
    PythonParseReport, analyze_python_project, discover_python_files, parse_python_files,
    relative_path_identifier,
};
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Parser)]
#[command(
    name = "spelunking",
    about = "Inspect Python and Django project structure"
)]
struct Cli {
    /// Target project directory to inspect.
    target: PathBuf,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Summary)]
    format: OutputFormat,

    /// Write output to a file instead of stdout. Use '-' for stdout.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Print each discovered Python file after the summary.
    #[arg(long)]
    list_files: bool,

    /// Include only these node types. Repeat the flag or use comma-separated values.
    #[arg(long = "node-type", value_name = "TYPE", value_parser = parse_node_type, value_delimiter = ',')]
    node_types: Vec<NodeType>,

    /// Include only these edge types. Repeat the flag or use comma-separated values.
    #[arg(long = "edge-type", value_name = "TYPE", value_parser = parse_edge_type, value_delimiter = ',')]
    edge_types: Vec<EdgeType>,

    /// Include only nodes whose relative source path starts with this prefix.
    #[arg(long = "path-prefix", value_name = "PREFIX", value_delimiter = ',')]
    path_prefixes: Vec<String>,

    /// Remove nodes that have no edges after filters are applied.
    #[arg(long)]
    drop_isolated: bool,

    /// Return a non-zero exit code when any file cannot be read or parsed.
    #[arg(long)]
    fail_on_diagnostics: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Summary,
    Json,
    Dot,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let python_files = discover_python_files(&cli.target)?;
    let parse_report = parse_python_files(&python_files);
    let unfiltered_graph =
        analyze_python_project(&cli.target, &python_files, &parse_report.modules);
    let filter = graph_filter(&cli);
    let graph = unfiltered_graph.filtered(&filter);
    let mut output = output_writer(cli.output.as_deref())?;

    match cli.format {
        OutputFormat::Summary => write_summary(
            &mut output,
            &cli,
            &python_files,
            &parse_report,
            &unfiltered_graph,
            &graph,
        )?,
        OutputFormat::Json => write_json_export(
            &mut output,
            &cli,
            &python_files,
            &parse_report,
            &unfiltered_graph,
            &graph,
        )?,
        OutputFormat::Dot => write_dot(&mut output, &graph)?,
    }

    output.flush()?;

    if parse_report.has_diagnostics() {
        print_diagnostics(&parse_report.diagnostics);
    }

    if cli.fail_on_diagnostics && parse_report.has_diagnostics() {
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}

fn write_summary(
    output: &mut dyn Write,
    cli: &Cli,
    python_files: &[PathBuf],
    parse_report: &PythonParseReport,
    unfiltered_graph: &GraphExport,
    graph: &GraphExport,
) -> io::Result<()> {
    writeln!(output, "Target: {}", cli.target.display())?;
    writeln!(output, "Discovered Python files: {}", python_files.len())?;
    writeln!(
        output,
        "Parsed Python files: {}",
        parse_report.parsed_count()
    )?;
    writeln!(output, "Diagnostics: {}", parse_report.diagnostic_count())?;

    if has_filters(cli) {
        writeln!(
            output,
            "Unfiltered graph nodes: {}",
            unfiltered_graph.node_count()
        )?;
        writeln!(
            output,
            "Unfiltered graph edges: {}",
            unfiltered_graph.edge_count()
        )?;
    }

    writeln!(output, "Graph nodes: {}", graph.node_count())?;
    writeln!(output, "Graph edges: {}", graph.edge_count())?;
    writeln!(
        output,
        "Django apps: {}",
        graph.node_count_by_type(NodeType::App)
    )?;
    writeln!(
        output,
        "Django models: {}",
        graph.node_count_by_type(NodeType::Model)
    )?;
    writeln!(
        output,
        "Django managers: {}",
        graph.node_count_by_type(NodeType::Manager)
    )?;
    writeln!(
        output,
        "Django generic relations: {}",
        graph.node_count_by_type(NodeType::GenericRelation)
    )?;
    writeln!(
        output,
        "Django URLs: {}",
        graph.node_count_by_type(NodeType::Url)
    )?;
    writeln!(
        output,
        "Django views: {}",
        graph.node_count_by_type(NodeType::View)
    )?;
    writeln!(
        output,
        "Django serializers: {}",
        graph.node_count_by_type(NodeType::Serializer)
    )?;
    writeln!(
        output,
        "Django forms: {}",
        graph.node_count_by_type(NodeType::Form)
    )?;
    writeln!(
        output,
        "Django services: {}",
        graph.node_count_by_type(NodeType::Service)
    )?;
    writeln!(
        output,
        "Django middleware: {}",
        graph.node_count_by_type(NodeType::Middleware)
    )?;
    writeln!(
        output,
        "Django context processors: {}",
        graph.node_count_by_type(NodeType::ContextProcessor)
    )?;
    writeln!(
        output,
        "Django signal handlers: {}",
        graph.node_count_by_type(NodeType::Handler)
    )?;
    writeln!(
        output,
        "Django signals: {}",
        graph.node_count_by_type(NodeType::Signal)
    )?;
    writeln!(
        output,
        "Django tasks: {}",
        graph.node_count_by_type(NodeType::Task)
    )?;
    writeln!(
        output,
        "Model inheritance edges: {}",
        graph.edge_count_by_type(EdgeType::Inherits)
    )?;
    writeln!(
        output,
        "Call edges: {}",
        graph.edge_count_by_type(EdgeType::Calls)
    )?;
    writeln!(
        output,
        "Model relationship edges: {}",
        graph.edge_count_by_type(EdgeType::RelatesTo)
    )?;
    writeln!(
        output,
        "Reverse relationship edges: {}",
        graph.edge_count_by_type(EdgeType::ReverseRelatesTo)
    )?;
    writeln!(
        output,
        "Manager usage edges: {}",
        graph.edge_count_by_type(EdgeType::UsesManager)
    )?;
    writeln!(
        output,
        "URL route edges: {}",
        graph.edge_count_by_type(EdgeType::RoutesTo)
    )?;
    writeln!(
        output,
        "Serialization edges: {}",
        graph.edge_count_by_type(EdgeType::Serializes)
    )?;
    writeln!(
        output,
        "Query edges: {}",
        graph.edge_count_by_type(EdgeType::Queries)
    )?;
    writeln!(
        output,
        "Global hook intercept edges: {}",
        graph.edge_count_by_type(EdgeType::Intercepts)
    )?;
    writeln!(
        output,
        "Trigger edges: {}",
        graph.edge_count_by_type(EdgeType::Triggers)
    )?;

    if cli.list_files {
        writeln!(output)?;
        writeln!(output, "Python files:")?;

        for path in python_files {
            writeln!(output, "{}", path.display())?;
        }
    }

    Ok(())
}

fn write_json_export(
    output: &mut dyn Write,
    cli: &Cli,
    python_files: &[PathBuf],
    parse_report: &PythonParseReport,
    unfiltered_graph: &GraphExport,
    graph: &GraphExport,
) -> Result<(), serde_json::Error> {
    let export = JsonExport {
        schema_version: JSON_SCHEMA_VERSION,
        target: cli.target.display().to_string(),
        summary: JsonSummary {
            discovered_python_files: python_files.len(),
            parsed_python_files: parse_report.parsed_count(),
            diagnostic_count: parse_report.diagnostic_count(),
            total_nodes: unfiltered_graph.node_count(),
            total_edges: unfiltered_graph.edge_count(),
            exported_nodes: graph.node_count(),
            exported_edges: graph.edge_count(),
        },
        filters: JsonFilters::from_cli(cli),
        diagnostics: parse_report
            .diagnostics
            .iter()
            .map(|diagnostic| JsonDiagnostic::from_diagnostic(&cli.target, diagnostic))
            .collect(),
        nodes: &graph.nodes,
        edges: &graph.edges,
    };

    serde_json::to_writer_pretty(&mut *output, &export)?;
    writeln!(output).map_err(serde_json::Error::io)
}

fn write_dot(output: &mut dyn Write, graph: &GraphExport) -> io::Result<()> {
    writeln!(output, "digraph spelunking {{")?;
    writeln!(output, "  graph [rankdir=\"LR\"];")?;
    writeln!(
        output,
        "  node [fontname=\"Helvetica\", shape=\"box\", style=\"rounded\"];"
    )?;
    writeln!(output, "  edge [fontname=\"Helvetica\"];")?;

    for node in &graph.nodes {
        write_dot_node(output, node)?;
    }

    for edge in &graph.edges {
        write_dot_edge(output, edge)?;
    }

    writeln!(output, "}}")
}

fn write_dot_node(output: &mut dyn Write, node: &Node) -> io::Result<()> {
    let label = dot_node_label(node);
    let mut attributes = vec![
        dot_attribute("label", &label),
        dot_attribute("id", &node.id),
        dot_attribute("type", node.node_type.as_str()),
        dot_attribute("shape", dot_node_shape(node.node_type)),
    ];

    if let Some(path) = &node.path {
        attributes.push(dot_attribute("tooltip", path));
    }

    for (key, value) in &node.attributes {
        attributes.push(dot_attribute(
            &format!("data_{}", dot_attribute_name(key)),
            value,
        ));
    }

    writeln!(
        output,
        "  \"{}\" [{}];",
        dot_escape(&node.id),
        attributes.join(", ")
    )
}

fn write_dot_edge(output: &mut dyn Write, edge: &Edge) -> io::Result<()> {
    let mut attributes = vec![dot_attribute("label", &dot_edge_label(edge))];

    for (key, value) in &edge.attributes {
        attributes.push(dot_attribute(
            &format!("data_{}", dot_attribute_name(key)),
            value,
        ));
    }

    writeln!(
        output,
        "  \"{}\" -> \"{}\" [{}];",
        dot_escape(&edge.source),
        dot_escape(&edge.target),
        attributes.join(", ")
    )
}

fn dot_node_label(node: &Node) -> String {
    let mut lines = vec![node.label.clone(), node.node_type.as_str().to_owned()];

    for flag in ["abstract", "proxy"] {
        if node
            .attributes
            .get(flag)
            .is_some_and(|value| value == "true")
        {
            lines.push(flag.to_owned());
        }
    }

    lines.join("\n")
}

fn dot_edge_label(edge: &Edge) -> String {
    let mut parts = vec![edge.edge_type.as_str().to_owned()];

    for key in ["field", "kind", "through", "accessor"] {
        if let Some(value) = edge.attributes.get(key) {
            parts.push(format!("{key}: {value}"));
        }
    }

    parts.join("\n")
}

fn dot_node_shape(node_type: NodeType) -> &'static str {
    match node_type {
        NodeType::SourceFile => "note",
        NodeType::App => "component",
        NodeType::Manager => "folder",
        NodeType::GenericRelation => "diamond",
        NodeType::Url => "oval",
        NodeType::Signal => "diamond",
        NodeType::Task => "hexagon",
        NodeType::Middleware => "octagon",
        NodeType::ContextProcessor => "parallelogram",
        NodeType::Model
        | NodeType::View
        | NodeType::Serializer
        | NodeType::Form
        | NodeType::Service
        | NodeType::Handler => "box",
    }
}

fn dot_attribute(name: &str, value: &str) -> String {
    format!("{name}=\"{}\"", dot_escape(value))
}

fn dot_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            _ => vec![character],
        })
        .collect()
}

fn dot_attribute_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn output_writer(path: Option<&Path>) -> io::Result<Box<dyn Write>> {
    match path {
        Some(path) if path != Path::new("-") => {
            Ok(Box::new(io::BufWriter::new(File::create(path)?)))
        }
        _ => Ok(Box::new(io::stdout())),
    }
}

fn graph_filter(cli: &Cli) -> GraphFilter {
    GraphFilter {
        node_types: cli.node_types.iter().copied().collect::<HashSet<_>>(),
        edge_types: cli.edge_types.iter().copied().collect::<HashSet<_>>(),
        path_prefixes: cli
            .path_prefixes
            .iter()
            .map(|prefix| normalize_path_prefix(prefix))
            .filter(|prefix| !prefix.is_empty())
            .collect(),
        drop_isolated: cli.drop_isolated,
    }
}

fn has_filters(cli: &Cli) -> bool {
    !cli.node_types.is_empty()
        || !cli.edge_types.is_empty()
        || !cli.path_prefixes.is_empty()
        || cli.drop_isolated
}

fn normalize_path_prefix(prefix: &str) -> String {
    let mut normalized = prefix.trim().replace('\\', "/");

    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_owned();
    }

    normalized.trim_matches('/').to_owned()
}

fn parse_node_type(value: &str) -> Result<NodeType, String> {
    value.parse()
}

fn parse_edge_type(value: &str) -> Result<EdgeType, String> {
    value.parse()
}

fn print_diagnostics(diagnostics: &[PythonParseDiagnostic]) {
    const MAX_DIAGNOSTICS: usize = 20;

    eprintln!();
    eprintln!("Diagnostics:");

    for diagnostic in diagnostics.iter().take(MAX_DIAGNOSTICS) {
        match diagnostic.offset {
            Some(offset) => eprintln!(
                "- {:?}: {} at byte offset {offset}: {}",
                diagnostic.kind,
                diagnostic.path.display(),
                diagnostic.message
            ),
            None => eprintln!(
                "- {:?}: {}: {}",
                diagnostic.kind,
                diagnostic.path.display(),
                diagnostic.message
            ),
        }
    }

    if diagnostics.len() > MAX_DIAGNOSTICS {
        eprintln!(
            "- ... {} more diagnostics omitted",
            diagnostics.len() - MAX_DIAGNOSTICS
        );
    }
}

#[derive(Serialize)]
struct JsonExport<'a> {
    schema_version: u32,
    target: String,
    summary: JsonSummary,
    filters: JsonFilters,
    diagnostics: Vec<JsonDiagnostic>,
    nodes: &'a [Node],
    edges: &'a [Edge],
}

#[derive(Serialize)]
struct JsonSummary {
    discovered_python_files: usize,
    parsed_python_files: usize,
    diagnostic_count: usize,
    total_nodes: usize,
    total_edges: usize,
    exported_nodes: usize,
    exported_edges: usize,
}

#[derive(Serialize)]
struct JsonFilters {
    node_types: Vec<String>,
    edge_types: Vec<String>,
    path_prefixes: Vec<String>,
    drop_isolated: bool,
}

impl JsonFilters {
    fn from_cli(cli: &Cli) -> Self {
        Self {
            node_types: sorted_type_names(&cli.node_types),
            edge_types: sorted_type_names(&cli.edge_types),
            path_prefixes: cli
                .path_prefixes
                .iter()
                .map(|prefix| normalize_path_prefix(prefix))
                .filter(|prefix| !prefix.is_empty())
                .collect(),
            drop_isolated: cli.drop_isolated,
        }
    }
}

#[derive(Serialize)]
struct JsonDiagnostic {
    path: String,
    kind: &'static str,
    message: String,
    offset: Option<u32>,
}

impl JsonDiagnostic {
    fn from_diagnostic(target: &Path, diagnostic: &PythonParseDiagnostic) -> Self {
        Self {
            path: relative_path_identifier(target, &diagnostic.path),
            kind: diagnostic.kind.as_str(),
            message: diagnostic.message.clone(),
            offset: diagnostic.offset,
        }
    }
}

fn sorted_type_names<T>(values: &[T]) -> Vec<String>
where
    T: ToString,
{
    let mut names = values.iter().map(ToString::to_string).collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_dot_attribute_values() {
        assert_eq!(
            dot_escape("quote\"backslash\\newline\n"),
            "quote\\\"backslash\\\\newline\\n"
        );
    }

    #[test]
    fn normalizes_path_prefixes_for_filtering() {
        assert_eq!(normalize_path_prefix("./shop/"), "shop");
    }
}

use clap::{Parser, ValueEnum};
use spelunking_core::{
    EdgeType, GraphExport, NodeType, PythonParseDiagnostic, analyze_python_project,
    discover_python_files, parse_python_files,
};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

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

    /// Print each discovered Python file after the summary.
    #[arg(long)]
    list_files: bool,

    /// Return a non-zero exit code when any file cannot be read or parsed.
    #[arg(long)]
    fail_on_diagnostics: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Summary,
    Json,
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
    let graph = analyze_python_project(&cli.target, &python_files, &parse_report.modules);

    match cli.format {
        OutputFormat::Summary => print_summary(&cli, &python_files, &parse_report, &graph),
        OutputFormat::Json => {
            let mut stdout = io::stdout().lock();

            serde_json::to_writer_pretty(&mut stdout, &graph)?;
            writeln!(stdout)?;
        }
    }

    if parse_report.has_diagnostics() {
        print_diagnostics(&parse_report.diagnostics);
    }

    if cli.fail_on_diagnostics && parse_report.has_diagnostics() {
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}

fn print_summary(
    cli: &Cli,
    python_files: &[PathBuf],
    parse_report: &spelunking_core::PythonParseReport,
    graph: &GraphExport,
) {
    println!("Target: {}", cli.target.display());
    println!("Discovered Python files: {}", python_files.len());
    println!("Parsed Python files: {}", parse_report.parsed_count());
    println!("Diagnostics: {}", parse_report.diagnostic_count());
    println!("Graph nodes: {}", graph.node_count());
    println!("Graph edges: {}", graph.edge_count());
    println!("Django apps: {}", graph.node_count_by_type(NodeType::App));
    println!(
        "Django models: {}",
        graph.node_count_by_type(NodeType::Model)
    );
    println!("Django URLs: {}", graph.node_count_by_type(NodeType::Url));
    println!("Django views: {}", graph.node_count_by_type(NodeType::View));
    println!(
        "Django serializers: {}",
        graph.node_count_by_type(NodeType::Serializer)
    );
    println!("Django forms: {}", graph.node_count_by_type(NodeType::Form));
    println!(
        "Django middleware: {}",
        graph.node_count_by_type(NodeType::Middleware)
    );
    println!(
        "Model inheritance edges: {}",
        graph.edge_count_by_type(EdgeType::Inherits)
    );
    println!(
        "Model relationship edges: {}",
        graph.edge_count_by_type(EdgeType::RelatesTo)
    );
    println!(
        "URL route edges: {}",
        graph.edge_count_by_type(EdgeType::RoutesTo)
    );
    println!(
        "Serialization edges: {}",
        graph.edge_count_by_type(EdgeType::Serializes)
    );
    println!(
        "Query edges: {}",
        graph.edge_count_by_type(EdgeType::Queries)
    );
    println!(
        "Middleware intercept edges: {}",
        graph.edge_count_by_type(EdgeType::Intercepts)
    );

    if cli.list_files {
        println!();
        println!("Python files:");

        for path in python_files {
            println!("{}", path.display());
        }
    }
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

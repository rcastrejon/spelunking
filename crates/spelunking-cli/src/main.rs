use clap::{Parser, ValueEnum};
use spelunking_core::{
    PythonParseDiagnostic, build_source_file_graph, discover_python_files, parse_python_files,
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

    match cli.format {
        OutputFormat::Summary => print_summary(&cli, &python_files, &parse_report),
        OutputFormat::Json => {
            let graph = build_source_file_graph(&cli.target, &python_files);
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
) {
    println!("Target: {}", cli.target.display());
    println!("Discovered Python files: {}", python_files.len());
    println!("Parsed Python files: {}", parse_report.parsed_count());
    println!("Diagnostics: {}", parse_report.diagnostic_count());

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

use clap::Parser;
use spelunking_core::{PythonParseDiagnostic, discover_python_files, parse_python_files};
use std::{path::PathBuf, process::ExitCode};

#[derive(Debug, Parser)]
#[command(
    name = "spelunking",
    about = "Inspect Python and Django project structure"
)]
struct Cli {
    /// Target project directory to inspect.
    target: PathBuf,

    /// Print each discovered Python file after the summary.
    #[arg(long)]
    list_files: bool,

    /// Return a non-zero exit code when any file cannot be read or parsed.
    #[arg(long)]
    fail_on_diagnostics: bool,
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

    println!("Target: {}", cli.target.display());
    println!("Discovered Python files: {}", python_files.len());
    println!("Parsed Python files: {}", parse_report.parsed_count());
    println!("Diagnostics: {}", parse_report.diagnostic_count());

    if cli.list_files {
        println!();
        println!("Python files:");

        for path in &python_files {
            println!("{}", path.display());
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

use clap::{Parser, ValueEnum};
use serde::Serialize;
use spelunking_core::{
    DjangoArtifactBundle, DjangoBehaviorReport, DjangoDomainFact, DjangoGuidanceReport,
    DjangoSubjectReport, Edge, EdgeType, GraphExport, GraphFilter, Node, NodeType,
    PythonParseDiagnostic, PythonParseReport, analyze_python_project, build_django_artifact_bundle,
    discover_python_files, django_subject_slug, inspect_django_behavior, inspect_django_guidance,
    inspect_django_subject, parse_python_files, relative_path_identifier,
    render_django_domain_facts_jsonl,
};
use std::{
    collections::HashSet,
    fs::{self, File},
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

    /// Inspect a Django model field, for example Reservation.status.
    #[arg(long = "inspect-subject", value_name = "MODEL.FIELD")]
    inspect_subject: Option<String>,

    /// Inspect mutation sites and behavior paths for a Django model field.
    #[arg(long = "inspect-behavior", value_name = "MODEL.FIELD")]
    inspect_behavior: Option<String>,

    /// Inspect operational risks, open questions, reading path, and related tests for a Django model field.
    #[arg(long = "inspect-guidance", value_name = "MODEL.FIELD")]
    inspect_guidance: Option<String>,

    /// Extract candidate domain facts for a Django model field.
    #[arg(long = "inspect-domain-facts", value_name = "MODEL.FIELD")]
    inspect_domain_facts: Option<String>,

    /// Generate the JSON evidence pack for a Django model field under the artifact directory.
    #[arg(long = "generate-evidence-pack", value_name = "MODEL.FIELD")]
    generate_evidence_pack: Option<String>,

    /// Generate JSONL candidate domain facts for a Django model field under the artifact directory.
    #[arg(long = "generate-domain-facts", value_name = "MODEL.FIELD")]
    generate_domain_facts: Option<String>,

    /// Generate the Markdown lifecycle report for a Django model field under the artifact directory.
    #[arg(long = "generate-report", value_name = "MODEL.FIELD")]
    generate_report: Option<String>,

    /// Generate the Markdown evaluation scorecard for a Django model field under the artifact directory.
    #[arg(long = "generate-evaluation", value_name = "MODEL.FIELD")]
    generate_evaluation: Option<String>,

    /// Generate evidence pack, Markdown report, and evaluation scorecard together.
    #[arg(long = "generate-artifacts", value_name = "MODEL.FIELD")]
    generate_artifacts: Option<String>,

    /// Directory for generated subject artifacts, relative to the target project unless absolute.
    #[arg(
        long = "artifact-dir",
        value_name = "PATH",
        default_value = ".domain-atlas"
    )]
    artifact_dir: PathBuf,

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
    let inspect_modes = [
        cli.inspect_subject.is_some(),
        cli.inspect_behavior.is_some(),
        cli.inspect_guidance.is_some(),
        cli.inspect_domain_facts.is_some(),
        cli.generate_evidence_pack.is_some(),
        cli.generate_domain_facts.is_some(),
        cli.generate_report.is_some(),
        cli.generate_evaluation.is_some(),
        cli.generate_artifacts.is_some(),
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count();

    if inspect_modes > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--inspect-subject, --inspect-behavior, --inspect-guidance, --inspect-domain-facts, and artifact generation flags cannot be used together",
        )
        .into());
    }

    let python_files = discover_python_files(&cli.target)?;
    let parse_report = parse_python_files(&python_files);
    let mut output = output_writer(cli.output.as_deref())?;

    if let Some(subject) = &cli.generate_artifacts {
        ensure_artifact_summary_format(cli.format, "--generate-artifacts")?;
        let bundle = build_django_artifact_bundle(&cli.target, &parse_report.modules, subject)?;
        let generated = write_django_artifacts(&cli, subject, &bundle, ArtifactSelection::All)?;
        write_generated_artifacts_summary(&mut output, &generated)?;
    } else if let Some(subject) = &cli.generate_evidence_pack {
        ensure_artifact_summary_format(cli.format, "--generate-evidence-pack")?;
        let bundle = build_django_artifact_bundle(&cli.target, &parse_report.modules, subject)?;
        let generated =
            write_django_artifacts(&cli, subject, &bundle, ArtifactSelection::EvidencePack)?;
        write_generated_artifacts_summary(&mut output, &generated)?;
    } else if let Some(subject) = &cli.generate_domain_facts {
        ensure_artifact_summary_format(cli.format, "--generate-domain-facts")?;
        let bundle = build_django_artifact_bundle(&cli.target, &parse_report.modules, subject)?;
        let generated =
            write_django_artifacts(&cli, subject, &bundle, ArtifactSelection::DomainFacts)?;
        write_generated_artifacts_summary(&mut output, &generated)?;
    } else if let Some(subject) = &cli.generate_report {
        ensure_artifact_summary_format(cli.format, "--generate-report")?;
        let bundle = build_django_artifact_bundle(&cli.target, &parse_report.modules, subject)?;
        let generated = write_django_artifacts(&cli, subject, &bundle, ArtifactSelection::Report)?;
        write_generated_artifacts_summary(&mut output, &generated)?;
    } else if let Some(subject) = &cli.generate_evaluation {
        ensure_artifact_summary_format(cli.format, "--generate-evaluation")?;
        let bundle = build_django_artifact_bundle(&cli.target, &parse_report.modules, subject)?;
        let generated =
            write_django_artifacts(&cli, subject, &bundle, ArtifactSelection::Evaluation)?;
        write_generated_artifacts_summary(&mut output, &generated)?;
    } else if let Some(subject) = &cli.inspect_guidance {
        let report = inspect_django_guidance(&cli.target, &parse_report.modules, subject)?;

        match cli.format {
            OutputFormat::Summary => write_guidance_summary(&mut output, &report)?,
            OutputFormat::Json => write_guidance_json(&mut output, &report)?,
            OutputFormat::Dot => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--inspect-guidance supports --format summary or --format json",
                )
                .into());
            }
        }
    } else if let Some(subject) = &cli.inspect_domain_facts {
        let bundle = build_django_artifact_bundle(&cli.target, &parse_report.modules, subject)?;

        match cli.format {
            OutputFormat::Summary => write_domain_facts_summary(&mut output, &bundle.domain_facts)?,
            OutputFormat::Json => write_domain_facts_json(&mut output, &bundle.domain_facts)?,
            OutputFormat::Dot => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--inspect-domain-facts supports --format summary or --format json",
                )
                .into());
            }
        }
    } else if let Some(subject) = &cli.inspect_behavior {
        let report = inspect_django_behavior(&cli.target, &parse_report.modules, subject)?;

        match cli.format {
            OutputFormat::Summary => write_behavior_summary(&mut output, &report)?,
            OutputFormat::Json => write_behavior_json(&mut output, &report)?,
            OutputFormat::Dot => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--inspect-behavior supports --format summary or --format json",
                )
                .into());
            }
        }
    } else if let Some(subject) = &cli.inspect_subject {
        let report = inspect_django_subject(&cli.target, &parse_report.modules, subject)?;

        match cli.format {
            OutputFormat::Summary => write_subject_summary(&mut output, &report)?,
            OutputFormat::Json => write_subject_json(&mut output, &report)?,
            OutputFormat::Dot => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--inspect-subject supports --format summary or --format json",
                )
                .into());
            }
        }
    } else {
        let unfiltered_graph =
            analyze_python_project(&cli.target, &python_files, &parse_report.modules);
        let filter = graph_filter(&cli);
        let graph = unfiltered_graph.filtered(&filter);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactSelection {
    EvidencePack,
    DomainFacts,
    Report,
    Evaluation,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedArtifact {
    label: &'static str,
    path: PathBuf,
}

fn ensure_artifact_summary_format(
    format: OutputFormat,
    flag: &'static str,
) -> Result<(), Box<dyn std::error::Error>> {
    if format == OutputFormat::Summary {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{flag} writes fixed artifact formats and supports only --format summary"),
        )
        .into())
    }
}

fn write_django_artifacts(
    cli: &Cli,
    subject: &str,
    bundle: &DjangoArtifactBundle,
    selection: ArtifactSelection,
) -> Result<Vec<GeneratedArtifact>, Box<dyn std::error::Error>> {
    let base_dir = artifact_base_dir(&cli.target, &cli.artifact_dir);
    let slug = django_subject_slug(subject);
    let mut generated = Vec::new();

    if matches!(
        selection,
        ArtifactSelection::EvidencePack | ArtifactSelection::All
    ) {
        let path = base_dir.join("evidence-packs").join(format!("{slug}.json"));
        write_json_file(&path, &bundle.evidence_pack)?;
        generated.push(GeneratedArtifact {
            label: "Evidence pack",
            path,
        });
    }

    if matches!(
        selection,
        ArtifactSelection::DomainFacts | ArtifactSelection::All
    ) {
        let path = base_dir
            .join("facts")
            .join(format!("{slug}-domain-facts.jsonl"));
        let contents = render_django_domain_facts_jsonl(&bundle.domain_facts)?;
        write_text_file(&path, &contents)?;
        generated.push(GeneratedArtifact {
            label: "Domain facts",
            path,
        });
    }

    if matches!(
        selection,
        ArtifactSelection::Report | ArtifactSelection::All
    ) {
        let path = base_dir
            .join("reports")
            .join(format!("{slug}-lifecycle.md"));
        write_text_file(&path, &bundle.markdown_report)?;
        generated.push(GeneratedArtifact {
            label: "Markdown report",
            path,
        });
    }

    if matches!(
        selection,
        ArtifactSelection::Evaluation | ArtifactSelection::All
    ) {
        let path = base_dir
            .join("evaluation")
            .join(format!("{slug}-evaluation.md"));
        write_text_file(&path, &bundle.evaluation_report)?;
        generated.push(GeneratedArtifact {
            label: "Evaluation scorecard",
            path,
        });
    }

    Ok(generated)
}

fn artifact_base_dir(target: &Path, artifact_dir: &Path) -> PathBuf {
    if artifact_dir.is_absolute() {
        artifact_dir.to_path_buf()
    } else {
        target.join(artifact_dir)
    }
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(path)?;
    let mut writer = io::BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writeln!(writer)?;
    Ok(())
}

fn write_text_file(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, contents)
}

fn write_generated_artifacts_summary(
    output: &mut dyn Write,
    generated: &[GeneratedArtifact],
) -> io::Result<()> {
    writeln!(output, "Generated:")?;

    for artifact in generated {
        writeln!(output, "- {}: {}", artifact.label, artifact.path.display())?;
    }

    Ok(())
}

fn write_subject_summary(output: &mut dyn Write, report: &DjangoSubjectReport) -> io::Result<()> {
    writeln!(output, "Subject: {}", report.subject)?;

    let Some(model) = &report.model else {
        writeln!(output)?;
        writeln!(output, "Model: not found")?;
        writeln!(output, "Confidence: {}", report.confidence)?;
        return Ok(());
    };

    writeln!(output)?;
    writeln!(output, "Model:")?;
    writeln!(output, "- {}", model.name)?;
    writeln!(output, "- Defined in {}:{}", model.path, model.line)?;
    writeln!(output, "- Confidence: {}", model.confidence)?;

    writeln!(output)?;
    writeln!(output, "Lifecycle candidate:")?;
    if let Some(candidate) = &report.lifecycle_candidate {
        writeln!(
            output,
            "- Field: {} ({})",
            candidate.field, candidate.field_type
        )?;

        if candidate.states.is_empty() {
            writeln!(output, "- States detected: none")?;
        } else {
            writeln!(
                output,
                "- States detected: {}",
                candidate
                    .states
                    .iter()
                    .map(|state| state.value.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }

        writeln!(output, "- Evidence: {}:{}", model.path, candidate.line)?;
        writeln!(output, "- Confidence: {}", candidate.confidence)?;
    } else {
        writeln!(output, "- Field not found")?;
    }

    writeln!(output)?;
    writeln!(output, "Primary fields:")?;
    write_limited_items(output, &report.fields, 12, |output, field| {
        let marker = if field.is_subject { " subject" } else { "" };
        writeln!(
            output,
            "- {}: {} ({}:{}){}",
            field.name, field.field_type, field.path, field.line, marker
        )
    })?;

    writeln!(output)?;
    writeln!(output, "Related models:")?;
    write_limited_items(
        output,
        &report.related_models,
        12,
        |output, relationship| {
            writeln!(
                output,
                "- {} via {} {} ({}:{})",
                relationship.model,
                relationship.relationship,
                relationship.field,
                relationship.path,
                relationship.line
            )
        },
    )?;

    writeln!(output)?;
    writeln!(output, "Relevant methods:")?;
    write_limited_items(output, &report.relevant_methods, 12, |output, method| {
        writeln!(
            output,
            "- {} ({}:{}) - {}",
            method.name, method.path, method.line, method.reason
        )
    })?;

    writeln!(output)?;
    writeln!(output, "Related Django components:")?;
    write_limited_items(
        output,
        &report.related_components,
        12,
        |output, component| {
            writeln!(
                output,
                "- {} {} ({}:{}) - {}",
                component.kind, component.name, component.path, component.line, component.reason
            )
        },
    )?;

    writeln!(output)?;
    writeln!(output, "Evidence:")?;
    write_limited_items(output, &report.evidence, 16, |output, evidence| {
        writeln!(
            output,
            "- {}:{} {}",
            evidence.path, evidence.line, evidence.detail
        )
    })?;

    writeln!(output)?;
    writeln!(output, "Confidence: {}", report.confidence)
}

fn write_subject_json(
    output: &mut dyn Write,
    report: &DjangoSubjectReport,
) -> Result<(), serde_json::Error> {
    serde_json::to_writer_pretty(&mut *output, report)?;
    writeln!(output).map_err(serde_json::Error::io)
}

fn write_behavior_summary(output: &mut dyn Write, report: &DjangoBehaviorReport) -> io::Result<()> {
    writeln!(output, "Behavior map: {}", report.subject)?;

    let Some(model) = &report.model else {
        writeln!(output)?;
        writeln!(output, "Model: not found")?;
        writeln!(output, "Confidence: {}", report.confidence)?;
        return Ok(());
    };

    writeln!(output)?;
    writeln!(output, "Model:")?;
    writeln!(output, "- {}", model.name)?;
    writeln!(output, "- Defined in {}:{}", model.path, model.line)?;

    writeln!(output)?;
    writeln!(output, "Mutation sites:")?;
    write_limited_items(output, &report.mutation_sites, 20, |output, site| {
        writeln!(
            output,
            "- {} in {} {} ({}:{})",
            site.kind, site.container_kind, site.container_name, site.path, site.line
        )?;
        writeln!(output, "  {}", site.mutation)?;
        writeln!(output, "  Confidence: {}", site.confidence)
    })?;

    writeln!(output)?;
    writeln!(output, "Behavior paths:")?;
    write_limited_items(output, &report.behavior_paths, 12, |output, path| {
        writeln!(output, "- {} ({})", path.kind, path.confidence)?;

        for step in &path.steps {
            writeln!(
                output,
                "  -> {} {} ({}:{})",
                step.kind, step.name, step.path, step.line
            )?;
        }

        Ok(())
    })?;

    writeln!(output)?;
    writeln!(output, "Evidence:")?;
    write_limited_items(output, &report.evidence, 20, |output, evidence| {
        writeln!(
            output,
            "- {}:{} {}",
            evidence.path, evidence.line, evidence.detail
        )
    })?;

    writeln!(output)?;
    writeln!(output, "Confidence: {}", report.confidence)
}

fn write_behavior_json(
    output: &mut dyn Write,
    report: &DjangoBehaviorReport,
) -> Result<(), serde_json::Error> {
    serde_json::to_writer_pretty(&mut *output, report)?;
    writeln!(output).map_err(serde_json::Error::io)
}

fn write_guidance_summary(output: &mut dyn Write, report: &DjangoGuidanceReport) -> io::Result<()> {
    writeln!(output, "Guidance for {}", report.subject)?;

    writeln!(output)?;
    writeln!(output, "Analysis basis:")?;
    writeln!(output, "- Scope: {}", report.analysis_basis.scope)?;
    writeln!(
        output,
        "- Subject slice: model={}, lifecycle={}, components={}, mutations={}, paths={}, tests={}, evidence={}",
        yes_no(report.analysis_basis.subject_slice.model_found),
        yes_no(
            report
                .analysis_basis
                .subject_slice
                .lifecycle_candidate_found
        ),
        report.analysis_basis.subject_slice.related_components,
        report.analysis_basis.subject_slice.mutation_sites,
        report.analysis_basis.subject_slice.behavior_paths,
        report.analysis_basis.subject_slice.related_tests,
        report.analysis_basis.subject_slice.evidence_items,
    )?;
    writeln!(output, "- Data sources:")?;
    write_limited_items(
        output,
        &report.analysis_basis.data_sources,
        6,
        |output, source| writeln!(output, "  - {source}"),
    )?;
    writeln!(output, "- Caveats:")?;
    write_limited_items(
        output,
        &report.analysis_basis.caveats,
        6,
        |output, caveat| writeln!(output, "  - {caveat}"),
    )?;

    writeln!(output)?;
    writeln!(output, "Risks:")?;
    write_limited_items(output, &report.risks, 10, |output, risk| {
        writeln!(
            output,
            "- {} ({}, {})",
            risk.title, risk.severity, risk.confidence
        )?;
        writeln!(output, "  {}", risk.description)?;

        for evidence in risk.evidence.iter().take(3) {
            writeln!(
                output,
                "  Evidence: {}:{} {}",
                evidence.path, evidence.line, evidence.detail
            )?;
        }

        Ok(())
    })?;

    writeln!(output)?;
    writeln!(output, "Open questions:")?;
    write_limited_items(output, &report.open_questions, 8, |output, question| {
        writeln!(output, "- {}", question.question)?;
        writeln!(output, "  {}", question.reason)
    })?;

    writeln!(output)?;
    writeln!(output, "Coupling signals:")?;
    write_limited_items(output, &report.coupling_signals, 8, |output, signal| {
        writeln!(
            output,
            "- {} ({}) - {}",
            signal.kind, signal.confidence, signal.description
        )
    })?;

    writeln!(output)?;
    writeln!(output, "Recommended reading path:")?;
    write_limited_items(output, &report.reading_path, 10, |output, entry| {
        writeln!(
            output,
            "{}. {}:{} ({})",
            entry.priority, entry.path, entry.line, entry.confidence
        )?;
        writeln!(output, "   {}", entry.reason)
    })?;

    writeln!(output)?;
    writeln!(output, "Related tests:")?;
    write_limited_items(output, &report.related_tests, 8, |output, test| {
        writeln!(
            output,
            "- {}:{} ({}) - {}",
            test.path, test.line, test.confidence, test.reason
        )
    })?;

    writeln!(output)?;
    writeln!(output, "Evidence:")?;
    write_limited_items(output, &report.evidence, 20, |output, evidence| {
        writeln!(
            output,
            "- {}:{} {}",
            evidence.path, evidence.line, evidence.detail
        )
    })?;

    writeln!(output)?;
    writeln!(output, "Confidence: {}", report.confidence)
}

fn write_guidance_json(
    output: &mut dyn Write,
    report: &DjangoGuidanceReport,
) -> Result<(), serde_json::Error> {
    serde_json::to_writer_pretty(&mut *output, report)?;
    writeln!(output).map_err(serde_json::Error::io)
}

fn write_domain_facts_summary(
    output: &mut dyn Write,
    facts: &[DjangoDomainFact],
) -> io::Result<()> {
    if facts.is_empty() {
        writeln!(output, "Domain facts proposed: none")?;
        return Ok(());
    }

    writeln!(output, "Domain facts proposed: {}", facts.len())?;

    for fact in facts {
        writeln!(output)?;
        writeln!(
            output,
            "- {} ({}, {}, {}, {})",
            fact.statement, fact.fact_type, fact.basis, fact.status, fact.confidence
        )?;
        writeln!(output, "  Subject: {}", fact.subject)?;
        writeln!(output, "  Origin: {}", fact.origin)?;
        writeln!(output, "  Rationale: {}", fact.rationale)?;

        for evidence in fact.evidence.iter().take(3) {
            writeln!(
                output,
                "  Evidence: {}:{} {}",
                evidence.path, evidence.line, evidence.detail
            )?;
        }

        if fact.evidence.len() > 3 {
            writeln!(output, "  Evidence: ... {} more", fact.evidence.len() - 3)?;
        }
    }

    Ok(())
}

fn write_domain_facts_json(
    output: &mut dyn Write,
    facts: &[DjangoDomainFact],
) -> Result<(), serde_json::Error> {
    serde_json::to_writer_pretty(&mut *output, facts)?;
    writeln!(output).map_err(serde_json::Error::io)
}

fn write_limited_items<T>(
    output: &mut dyn Write,
    values: &[T],
    limit: usize,
    mut write_item: impl FnMut(&mut dyn Write, &T) -> io::Result<()>,
) -> io::Result<()> {
    if values.is_empty() {
        writeln!(output, "- none")?;
        return Ok(());
    }

    for value in values.iter().take(limit) {
        write_item(output, value)?;
    }

    if values.len() > limit {
        writeln!(output, "- ... {} more omitted", values.len() - limit)?;
    }

    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
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

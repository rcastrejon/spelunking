use super::subject::{
    DjangoBehaviorPath, DjangoBehaviorReport, DjangoCouplingSignal, DjangoGuidanceBasis,
    DjangoGuidanceReport, DjangoLifecycleCandidate, DjangoMutationSite, DjangoOpenQuestion,
    DjangoReadingPathEntry, DjangoRelatedComponent, DjangoRelatedModel, DjangoRelatedTest,
    DjangoRiskSignal, DjangoSubjectError, DjangoSubjectEvidence, DjangoSubjectModel,
    DjangoSubjectReport, DjangoSubjectState, inspect_django_behavior, inspect_django_guidance,
    inspect_django_subject,
};
use crate::parsing::ParsedPythonModule;
use serde::Serialize;
use std::path::Path;

pub const DJANGO_EVIDENCE_PACK_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoEvidencePack {
    pub schema_version: u32,
    pub subject: String,
    pub lifecycle: DjangoEvidenceLifecycle,
    pub relationship_map: DjangoEvidenceRelationshipMap,
    pub mutation_sites: Vec<DjangoMutationSite>,
    pub behavior_paths: Vec<DjangoBehaviorPath>,
    pub risk_signals: Vec<DjangoRiskSignal>,
    pub open_questions: Vec<DjangoOpenQuestion>,
    pub reading_path: Vec<DjangoReadingPathEntry>,
    pub related_tests: Vec<DjangoRelatedTest>,
    pub coupling_signals: Vec<DjangoCouplingSignal>,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub analysis_basis: DjangoGuidanceBasis,
    pub confidence: DjangoEvidenceConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoEvidenceLifecycle {
    pub model: Option<DjangoSubjectModel>,
    pub field: Option<String>,
    pub field_type: Option<String>,
    pub states: Vec<DjangoSubjectState>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoEvidenceRelationshipMap {
    pub related_models: Vec<DjangoRelatedModel>,
    pub related_components: Vec<DjangoRelatedComponent>,
    pub behavior_paths: Vec<DjangoBehaviorPath>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoEvidenceConfidence {
    pub subject: String,
    pub behavior: String,
    pub guidance: String,
    pub overall: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DjangoArtifactBundle {
    pub evidence_pack: DjangoEvidencePack,
    pub markdown_report: String,
    pub evaluation_report: String,
}

pub fn build_django_evidence_pack(
    root: impl AsRef<Path>,
    modules: &[ParsedPythonModule],
    subject: &str,
) -> Result<DjangoEvidencePack, DjangoSubjectError> {
    let root = root.as_ref();
    let subject_report = inspect_django_subject(root, modules, subject)?;
    let behavior_report = inspect_django_behavior(root, modules, subject)?;
    let guidance_report = inspect_django_guidance(root, modules, subject)?;

    Ok(evidence_pack_from_reports(
        subject_report,
        behavior_report,
        guidance_report,
    ))
}

pub fn build_django_artifact_bundle(
    root: impl AsRef<Path>,
    modules: &[ParsedPythonModule],
    subject: &str,
) -> Result<DjangoArtifactBundle, DjangoSubjectError> {
    let evidence_pack = build_django_evidence_pack(root, modules, subject)?;
    let markdown_report = render_django_markdown_report(&evidence_pack);
    let evaluation_report = render_django_evaluation_report(&evidence_pack);

    Ok(DjangoArtifactBundle {
        evidence_pack,
        markdown_report,
        evaluation_report,
    })
}

pub fn render_django_markdown_report(pack: &DjangoEvidencePack) -> String {
    let mut report = String::new();

    push_line(&mut report, &format!("# {} Lifecycle Report", pack.subject));
    push_line(&mut report, "");
    push_line(
        &mut report,
        &format!(
            "Spelunking generated this report from a subject-focused behavioral slice. Overall confidence: **{}**.",
            pack.confidence.overall
        ),
    );

    push_line(&mut report, "");
    push_line(&mut report, "## Lifecycle Summary");
    if let Some(model) = &pack.lifecycle.model {
        push_line(
            &mut report,
            &format!(
                "- Model: `{}` in `{}`:{}",
                model.name, model.path, model.line
            ),
        );
    } else {
        push_line(&mut report, "- Model: not found");
    }

    if let Some(field) = &pack.lifecycle.field {
        push_line(
            &mut report,
            &format!(
                "- Field: `{}` ({})",
                field,
                pack.lifecycle.field_type.as_deref().unwrap_or("unknown")
            ),
        );
    } else {
        push_line(&mut report, "- Field: not found");
    }

    push_line(
        &mut report,
        &format!(
            "- States detected: {}",
            state_values(&pack.lifecycle.states)
        ),
    );

    push_line(&mut report, "");
    push_line(&mut report, "## Relationship Map");
    push_limited(
        &mut report,
        &pack.relationship_map.related_models,
        8,
        |relationship| {
            format!(
                "- `{}` via `{}` {} (`{}`:{})",
                relationship.model,
                relationship.field,
                relationship.relationship,
                relationship.path,
                relationship.line
            )
        },
    );
    push_limited(
        &mut report,
        &pack.relationship_map.related_components,
        8,
        |component| {
            format!(
                "- {} `{}` (`{}`:{}) - {}",
                component.kind, component.name, component.path, component.line, component.reason
            )
        },
    );

    push_line(&mut report, "");
    push_line(&mut report, "## Mutation Sites");
    push_limited(&mut report, &pack.mutation_sites, 12, |site| {
        format!(
            "- {} in {} `{}` (`{}`:{}) - `{}`",
            site.kind,
            site.container_kind,
            site.container_name,
            site.path,
            site.line,
            site.mutation
        )
    });

    push_line(&mut report, "");
    push_line(&mut report, "## Behavior Paths");
    if pack.behavior_paths.is_empty() {
        push_line(&mut report, "- none");
    } else {
        for path in pack.behavior_paths.iter().take(8) {
            push_line(
                &mut report,
                &format!("- {} ({})", path.kind, path.confidence),
            );
            for step in &path.steps {
                push_line(
                    &mut report,
                    &format!(
                        "  - {} `{}` (`{}`:{})",
                        step.kind, step.name, step.path, step.line
                    ),
                );
            }
        }
    }

    push_line(&mut report, "");
    push_line(&mut report, "## Risks");
    push_limited(&mut report, &pack.risk_signals, 10, |risk| {
        format!(
            "- **{}** ({}, {}) - {}",
            risk.title, risk.severity, risk.confidence, risk.description
        )
    });

    push_line(&mut report, "");
    push_line(&mut report, "## Open Questions");
    push_limited(&mut report, &pack.open_questions, 8, |question| {
        format!("- {} {}", question.question, question.reason)
    });

    push_line(&mut report, "");
    push_line(&mut report, "## Recommended Reading Path");
    push_limited(&mut report, &pack.reading_path, 10, |entry| {
        format!(
            "{}. `{}`:{} - {}",
            entry.priority, entry.path, entry.line, entry.reason
        )
    });

    push_line(&mut report, "");
    push_line(&mut report, "## Related Tests");
    push_limited(&mut report, &pack.related_tests, 8, |test| {
        format!(
            "- `{}`:{} ({}) - {}",
            test.path, test.line, test.confidence, test.reason
        )
    });

    push_line(&mut report, "");
    push_line(&mut report, "## Evidence");
    push_limited(&mut report, &pack.evidence, 20, |evidence| {
        format!(
            "- `{}`:{} {}",
            evidence.path, evidence.line, evidence.detail
        )
    });

    push_line(&mut report, "");
    push_line(&mut report, "## Caveats");
    push_limited(&mut report, &pack.analysis_basis.caveats, 6, |caveat| {
        format!("- {caveat}")
    });

    report
}

pub fn render_django_evaluation_report(pack: &DjangoEvidencePack) -> String {
    let mut report = String::new();

    push_line(
        &mut report,
        &format!("# {} Evidence Pack Evaluation", pack.subject),
    );
    push_line(&mut report, "");
    push_line(
        &mut report,
        "Use this scorecard to compare manual exploration, a generic agent without the evidence pack, and an agent prompted with the evidence pack.",
    );

    push_line(&mut report, "");
    push_line(&mut report, "## Evidence Pack Baseline");
    push_line(
        &mut report,
        &format!("- Mutation sites captured: {}", pack.mutation_sites.len()),
    );
    push_line(
        &mut report,
        &format!("- Behavior paths captured: {}", pack.behavior_paths.len()),
    );
    push_line(
        &mut report,
        &format!("- Risk signals captured: {}", pack.risk_signals.len()),
    );
    push_line(
        &mut report,
        &format!("- Open questions captured: {}", pack.open_questions.len()),
    );
    push_line(
        &mut report,
        &format!(
            "- Reading path entries captured: {}",
            pack.reading_path.len()
        ),
    );
    push_line(
        &mut report,
        &format!("- Related tests captured: {}", pack.related_tests.len()),
    );

    push_line(&mut report, "");
    push_line(&mut report, "## Comparison Scorecard");
    push_line(
        &mut report,
        "| Scenario | Key files selected | Mutation sites found | Risks found | Questions generated | Exploration needed | Notes |",
    );
    push_line(&mut report, "| --- | --- | --- | --- | --- | --- | --- |");
    push_line(
        &mut report,
        "| Manual repo exploration | TBD | TBD | TBD | TBD | TBD | Fill after timed manual pass. |",
    );
    push_line(
        &mut report,
        "| Generic agent without evidence pack | TBD | TBD | TBD | TBD | TBD | Ask the same lifecycle question without Spelunking context. |",
    );
    push_line(
        &mut report,
        "| Agent with evidence pack | TBD | TBD | TBD | TBD | TBD | Prompt the agent with the JSON evidence pack first. |",
    );

    push_line(&mut report, "");
    push_line(&mut report, "## Expected Differentiators To Check");
    push_line(
        &mut report,
        &format!(
            "- File selection should start with: {}",
            reading_path_files(pack)
        ),
    );
    push_line(
        &mut report,
        &format!(
            "- Mutation-site recall should include: {}",
            mutation_site_summary(pack)
        ),
    );
    push_line(
        &mut report,
        &format!("- Risk coverage should include: {}", risk_summary(pack)),
    );

    push_line(&mut report, "");
    push_line(&mut report, "## Recommended Prompt");
    push_line(&mut report, "```text");
    push_line(
        &mut report,
        "Using the attached Spelunking evidence pack, explain this lifecycle, identify risky mutation paths, and recommend the files to read before changing it. Call out any uncertainty from the pack caveats.",
    );
    push_line(&mut report, "```");

    report
}

pub fn django_subject_slug(subject: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;

    for character in subject.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "subject".to_owned()
    } else {
        slug
    }
}

fn evidence_pack_from_reports(
    subject_report: DjangoSubjectReport,
    behavior_report: DjangoBehaviorReport,
    guidance_report: DjangoGuidanceReport,
) -> DjangoEvidencePack {
    let lifecycle = lifecycle_from_subject(&subject_report);
    let subject_confidence = subject_report.confidence.clone();
    let behavior_confidence = behavior_report.confidence.clone();
    let guidance_confidence = guidance_report.confidence.clone();
    let overall_confidence = confidence_from_parts(&[
        &lifecycle.confidence,
        &behavior_confidence,
        &guidance_confidence,
    ]);
    let relationship_map = DjangoEvidenceRelationshipMap {
        related_models: subject_report.related_models,
        related_components: subject_report.related_components,
        behavior_paths: behavior_report.behavior_paths.clone(),
        confidence: confidence_from_parts(&[
            &subject_confidence,
            &behavior_confidence,
            &guidance_confidence,
        ]),
    };
    let evidence = merged_evidence(
        subject_report.evidence,
        behavior_report.evidence,
        guidance_report.evidence.clone(),
    );
    let confidence = DjangoEvidenceConfidence {
        subject: subject_confidence,
        behavior: behavior_confidence,
        guidance: guidance_confidence,
        overall: overall_confidence,
    };

    DjangoEvidencePack {
        schema_version: DJANGO_EVIDENCE_PACK_SCHEMA_VERSION,
        subject: guidance_report.subject,
        lifecycle,
        relationship_map,
        mutation_sites: behavior_report.mutation_sites,
        behavior_paths: behavior_report.behavior_paths,
        risk_signals: guidance_report.risks,
        open_questions: guidance_report.open_questions,
        reading_path: guidance_report.reading_path,
        related_tests: guidance_report.related_tests,
        coupling_signals: guidance_report.coupling_signals,
        evidence,
        analysis_basis: guidance_report.analysis_basis,
        confidence,
    }
}

fn lifecycle_from_subject(report: &DjangoSubjectReport) -> DjangoEvidenceLifecycle {
    let candidate = report.lifecycle_candidate.as_ref();

    DjangoEvidenceLifecycle {
        model: report.model.clone(),
        field: candidate.map(|candidate| candidate.field.clone()),
        field_type: candidate.map(|candidate| candidate.field_type.clone()),
        states: candidate
            .map(|candidate| candidate.states.clone())
            .unwrap_or_default(),
        confidence: lifecycle_confidence(candidate, report.model.as_ref()),
    }
}

fn lifecycle_confidence(
    candidate: Option<&DjangoLifecycleCandidate>,
    model: Option<&DjangoSubjectModel>,
) -> String {
    match (candidate, model) {
        (Some(candidate), Some(_)) => candidate.confidence.clone(),
        (None, Some(_)) => "low".to_owned(),
        _ => "low".to_owned(),
    }
}

fn merged_evidence(
    subject_evidence: Vec<DjangoSubjectEvidence>,
    behavior_evidence: Vec<DjangoSubjectEvidence>,
    guidance_evidence: Vec<DjangoSubjectEvidence>,
) -> Vec<DjangoSubjectEvidence> {
    let mut evidence = subject_evidence
        .into_iter()
        .chain(behavior_evidence)
        .chain(guidance_evidence)
        .collect::<Vec<_>>();

    evidence.sort();
    evidence.dedup();
    evidence
}

fn confidence_from_parts(parts: &[&str]) -> String {
    if parts.iter().any(|part| *part == "low") {
        "low".to_owned()
    } else if parts.iter().all(|part| *part == "high") {
        "high".to_owned()
    } else {
        "medium".to_owned()
    }
}

fn state_values(states: &[DjangoSubjectState]) -> String {
    if states.is_empty() {
        "none".to_owned()
    } else {
        states
            .iter()
            .map(|state| format!("`{}`", state.value))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn reading_path_files(pack: &DjangoEvidencePack) -> String {
    if pack.reading_path.is_empty() {
        "none".to_owned()
    } else {
        pack.reading_path
            .iter()
            .take(5)
            .map(|entry| format!("`{}`", entry.path))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn mutation_site_summary(pack: &DjangoEvidencePack) -> String {
    if pack.mutation_sites.is_empty() {
        "none".to_owned()
    } else {
        pack.mutation_sites
            .iter()
            .take(5)
            .map(|site| format!("{} in `{}`", site.kind, site.path))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn risk_summary(pack: &DjangoEvidencePack) -> String {
    if pack.risk_signals.is_empty() {
        "none".to_owned()
    } else {
        pack.risk_signals
            .iter()
            .take(5)
            .map(|risk| risk.title.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn push_limited<T>(
    output: &mut String,
    values: &[T],
    limit: usize,
    mut render: impl FnMut(&T) -> String,
) {
    if values.is_empty() {
        push_line(output, "- none");
        return;
    }

    for value in values.iter().take(limit) {
        push_line(output, &render(value));
    }

    if values.len() > limit {
        push_line(
            output,
            &format!("- ... {} more omitted", values.len() - limit),
        );
    }
}

fn push_line(output: &mut String, line: &str) {
    output.push_str(line);
    output.push('\n');
}

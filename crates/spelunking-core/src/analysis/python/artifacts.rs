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
pub const DJANGO_DOMAIN_FACT_SCHEMA_VERSION: u32 = 1;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoDomainFact {
    pub schema_version: u32,
    pub id: String,
    pub statement: String,
    #[serde(rename = "type")]
    pub fact_type: String,
    pub subject: String,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub confidence: String,
    pub origin: String,
    pub basis: String,
    pub status: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DjangoArtifactBundle {
    pub evidence_pack: DjangoEvidencePack,
    pub domain_facts: Vec<DjangoDomainFact>,
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
    let domain_facts = extract_django_domain_facts(&evidence_pack);
    let markdown_report = render_django_markdown_report(&evidence_pack);
    let evaluation_report = render_django_evaluation_report(&evidence_pack);

    Ok(DjangoArtifactBundle {
        evidence_pack,
        domain_facts,
        markdown_report,
        evaluation_report,
    })
}

pub fn extract_django_domain_facts(pack: &DjangoEvidencePack) -> Vec<DjangoDomainFact> {
    let model_name = domain_model_name(pack);
    let field_name = domain_field_name(pack);
    let subject = pack.subject.clone();
    let mut facts = Vec::new();

    push_domain_fact(
        &mut facts,
        "domain_concept_candidate",
        subject.clone(),
        format!("{model_name} appears to be a domain concept in this behavior slice."),
        model_evidence(pack),
        pack.lifecycle.confidence.clone(),
        "programmatic",
        "observed",
        "The evidence pack found a Django model as the primary subject of the inspected behavior.",
    );

    if pack.lifecycle.field.is_some() {
        push_domain_fact(
            &mut facts,
            "lifecycle_candidate",
            subject.clone(),
            format!("{model_name} appears to have a lifecycle controlled by `{field_name}`."),
            lifecycle_evidence(pack),
            pack.lifecycle.confidence.clone(),
            "heuristic",
            "inferred",
            "A subject field with detected states or mutations is being treated as candidate lifecycle evidence.",
        );
    }

    if !pack.lifecycle.states.is_empty() {
        push_domain_fact(
            &mut facts,
            "lifecycle_candidate",
            subject.clone(),
            format!(
                "{model_name}.{field_name} exposes observed lifecycle states: {}.",
                state_values(&pack.lifecycle.states)
            ),
            state_evidence(pack),
            pack.lifecycle.confidence.clone(),
            "programmatic",
            "observed",
            "Lifecycle states were detected from model constants, choices, or nearby field evidence.",
        );
    }

    for relationship in &pack.relationship_map.related_models {
        push_domain_fact(
            &mut facts,
            "concept_relationship",
            subject.clone(),
            format!(
                "{model_name} is related to {} through `{}` ({}).",
                relationship.model, relationship.field, relationship.relationship
            ),
            vec![DjangoSubjectEvidence {
                path: relationship.path.clone(),
                line: relationship.line,
                detail: format!("relationship field {}", relationship.field),
            }],
            relationship.confidence.clone(),
            "programmatic",
            "observed",
            "A Django ORM relationship was detected on or near the subject model.",
        );
    }

    for site in &pack.mutation_sites {
        let mutation_evidence = vec![DjangoSubjectEvidence {
            path: site.path.clone(),
            line: site.line,
            detail: format!("{} mutation in {}", site.kind, site.container_name),
        }];
        let channel = mutation_channel(site);

        if let Some(state) = state_value_from_mutation(site, &pack.lifecycle.states) {
            push_domain_fact(
                &mut facts,
                "flow_step",
                subject.clone(),
                format!("{model_name}.{field_name} is set to `{state}` through {channel}."),
                mutation_evidence.clone(),
                site.confidence.clone(),
                "programmatic",
                "observed",
                "A mutation site writes a detected lifecycle state to the subject field.",
            );
            push_domain_fact(
                &mut facts,
                "business_rule_candidate",
                subject.clone(),
                transition_statement(&model_name, &state, site),
                mutation_evidence.clone(),
                site.confidence.clone(),
                "heuristic",
                "inferred",
                "A write to a lifecycle state suggests a candidate domain transition, but valid transition rules need human review.",
            );
        } else {
            push_domain_fact(
                &mut facts,
                "flow_step",
                subject.clone(),
                format!("{model_name}.{field_name} is mutated through {channel}."),
                mutation_evidence.clone(),
                site.confidence.clone(),
                "programmatic",
                "observed",
                "A mutation site writes to the subject field, even though no concrete lifecycle state was extracted.",
            );
        }

        if matches!(
            site.container_kind.as_str(),
            "task" | "signal_handler" | "webhook" | "management_command"
        ) {
            push_domain_fact(
                &mut facts,
                "side_effect",
                subject.clone(),
                format!(
                    "{model_name}.{field_name} can change outside the normal request path through {channel}."
                ),
                mutation_evidence,
                site.confidence.clone(),
                "heuristic",
                "inferred",
                "The mutation occurs in async, signal, webhook, or command code, so side effects and idempotency may need review.",
            );
        }
    }

    for signal in &pack.coupling_signals {
        push_domain_fact(
            &mut facts,
            "boundary_risk",
            subject.clone(),
            signal.description.clone(),
            signal.evidence.clone(),
            signal.confidence.clone(),
            "heuristic",
            "inferred",
            "A coupling signal indicates that the behavior may cross ownership or layer boundaries.",
        );
    }

    for risk in &pack.risk_signals {
        let fact_type = if risk.title.contains("Out-of-request") {
            "side_effect"
        } else {
            "boundary_risk"
        };
        push_domain_fact(
            &mut facts,
            fact_type,
            subject.clone(),
            risk.description.clone(),
            risk.evidence.clone(),
            risk.confidence.clone(),
            "heuristic",
            "inferred",
            &format!("Guidance raised this risk signal: {}.", risk.title),
        );
    }

    for question in &pack.open_questions {
        push_domain_fact(
            &mut facts,
            "open_question",
            subject.clone(),
            question.question.clone(),
            question.evidence.clone(),
            question.confidence.clone(),
            "heuristic",
            "inferred",
            &question.reason,
        );

        if let Some(statement) = pending_decision_statement(&model_name, &field_name, question) {
            push_domain_fact(
                &mut facts,
                "pending_decision",
                subject.clone(),
                statement,
                question.evidence.clone(),
                question.confidence.clone(),
                "heuristic",
                "inferred",
                "The open question implies a domain ownership or transition decision that should be reviewed by the team.",
            );
        }
    }

    push_domain_fact(
        &mut facts,
        "glossary_term_candidate",
        subject.clone(),
        format!("`{model_name}` is a candidate glossary term for this behavior slice."),
        model_evidence(pack),
        pack.lifecycle.confidence.clone(),
        "heuristic",
        "inferred",
        "The model name appears as the central noun in the evidence pack and should be validated against business language.",
    );

    if !pack.lifecycle.states.is_empty() {
        push_domain_fact(
            &mut facts,
            "glossary_term_candidate",
            subject,
            format!(
                "{} are candidate lifecycle vocabulary terms for {model_name}.{field_name}.",
                state_values(&pack.lifecycle.states)
            ),
            state_evidence(pack),
            pack.lifecycle.confidence.clone(),
            "heuristic",
            "inferred",
            "Detected states may represent ubiquitous language, but the business terms need confirmation.",
        );
    }

    finalize_domain_facts(&pack.subject, facts)
}

pub fn render_django_domain_facts_jsonl(
    facts: &[DjangoDomainFact],
) -> Result<String, serde_json::Error> {
    let mut output = String::new();

    for fact in facts {
        output.push_str(&serde_json::to_string(fact)?);
        output.push('\n');
    }

    Ok(output)
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

fn push_domain_fact(
    facts: &mut Vec<DjangoDomainFact>,
    fact_type: &str,
    subject: String,
    statement: String,
    evidence: Vec<DjangoSubjectEvidence>,
    confidence: String,
    origin: &str,
    basis: &str,
    rationale: &str,
) {
    if evidence.is_empty() {
        return;
    }

    facts.push(DjangoDomainFact {
        schema_version: DJANGO_DOMAIN_FACT_SCHEMA_VERSION,
        id: String::new(),
        statement,
        fact_type: fact_type.to_owned(),
        subject,
        evidence,
        confidence,
        origin: origin.to_owned(),
        basis: basis.to_owned(),
        status: "proposed".to_owned(),
        rationale: rationale.to_owned(),
    });
}

fn finalize_domain_facts(subject: &str, mut facts: Vec<DjangoDomainFact>) -> Vec<DjangoDomainFact> {
    facts.sort_by(|left, right| {
        left.fact_type
            .cmp(&right.fact_type)
            .then(left.statement.cmp(&right.statement))
    });
    facts.dedup_by(|left, right| {
        left.fact_type == right.fact_type && left.statement == right.statement
    });

    let slug = django_subject_slug(subject);
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.id = format!("{slug}-fact-{:03}", index + 1);
    }

    facts
}

fn domain_model_name(pack: &DjangoEvidencePack) -> String {
    pack.lifecycle
        .model
        .as_ref()
        .map(|model| model.name.clone())
        .unwrap_or_else(|| {
            pack.subject
                .split('.')
                .rev()
                .nth(1)
                .unwrap_or(&pack.subject)
                .to_owned()
        })
}

fn domain_field_name(pack: &DjangoEvidencePack) -> String {
    pack.lifecycle.field.clone().unwrap_or_else(|| {
        pack.subject
            .rsplit('.')
            .next()
            .unwrap_or("field")
            .to_owned()
    })
}

fn model_evidence(pack: &DjangoEvidencePack) -> Vec<DjangoSubjectEvidence> {
    pack.lifecycle
        .model
        .as_ref()
        .map(|model| {
            vec![DjangoSubjectEvidence {
                path: model.path.clone(),
                line: model.line,
                detail: format!("primary model {}", model.name),
            }]
        })
        .unwrap_or_default()
}

fn lifecycle_evidence(pack: &DjangoEvidencePack) -> Vec<DjangoSubjectEvidence> {
    let mut evidence = model_evidence(pack);
    evidence.extend(state_evidence(pack).into_iter().take(4));
    evidence
}

fn state_evidence(pack: &DjangoEvidencePack) -> Vec<DjangoSubjectEvidence> {
    pack.lifecycle
        .states
        .iter()
        .map(|state| DjangoSubjectEvidence {
            path: state.path.clone(),
            line: state.line,
            detail: format!("detected lifecycle state {}", state.value),
        })
        .collect()
}

fn state_value_from_mutation(
    site: &DjangoMutationSite,
    states: &[DjangoSubjectState],
) -> Option<String> {
    let value = site.value.as_deref().unwrap_or(&site.mutation);
    let normalized_value = domain_token(value);

    states
        .iter()
        .find(|state| {
            let normalized_state = domain_token(&state.value);
            !normalized_state.is_empty() && normalized_value.contains(&normalized_state)
        })
        .map(|state| state.value.clone())
}

fn domain_token(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn mutation_channel(site: &DjangoMutationSite) -> String {
    match site.container_kind.as_str() {
        "model_method" => format!("model method `{}`", site.container_name),
        "view" => format!("API view `{}`", site.container_name),
        "serializer" => format!("serializer `{}`", site.container_name),
        "form" => format!("form `{}`", site.container_name),
        "task" => format!("background task `{}`", site.container_name),
        "signal_handler" => format!("Django signal handler `{}`", site.container_name),
        "webhook" => format!("webhook `{}`", site.container_name),
        "admin_action" => format!("admin action `{}`", site.container_name),
        "management_command" => format!("management command `{}`", site.container_name),
        _ => format!("{} `{}`", site.container_kind, site.container_name),
    }
}

fn transition_statement(model_name: &str, state: &str, site: &DjangoMutationSite) -> String {
    match site.container_kind.as_str() {
        "model_method" => {
            format!("{model_name} has a model-level transition toward `{state}`.")
        }
        "view" => format!("{model_name} may become `{state}` through an API action."),
        "serializer" | "form" => format!(
            "{model_name} may become `{state}` during validation or request data processing."
        ),
        "task" => {
            format!("{model_name} may become `{state}` from background processing.")
        }
        "signal_handler" => {
            format!("{model_name} may become `{state}` from implicit Django signal behavior.")
        }
        "webhook" => {
            format!("{model_name} may become `{state}` from external webhook behavior.")
        }
        "admin_action" => {
            format!("{model_name} may be set to `{state}` manually through admin behavior.")
        }
        "management_command" => {
            format!("{model_name} may be set to `{state}` through a management command.")
        }
        _ => format!(
            "{model_name} may become `{state}` through {}.",
            mutation_channel(site)
        ),
    }
}

fn pending_decision_statement(
    model_name: &str,
    field_name: &str,
    question: &DjangoOpenQuestion,
) -> Option<String> {
    let normalized = question.question.to_ascii_lowercase();

    if normalized.contains("which module should own") {
        Some(format!(
            "Decide which module owns valid transitions for {model_name}.{field_name}."
        ))
    } else if normalized.contains("should this external app transition") {
        Some(format!(
            "Decide whether external apps may transition {model_name}.{field_name} directly."
        ))
    } else if normalized.contains("admin changes") || normalized.contains("admin cancellation") {
        Some(format!(
            "Decide whether admin changes to {model_name}.{field_name} must follow the same rules as API changes."
        ))
    } else {
        None
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

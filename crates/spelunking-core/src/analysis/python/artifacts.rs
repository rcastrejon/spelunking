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
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const DJANGO_EVIDENCE_PACK_SCHEMA_VERSION: u32 = 1;
pub const DJANGO_DOMAIN_FACT_SCHEMA_VERSION: u32 = 1;

/// Domain fact types emitted by the Increment 1 extractor.
pub const DJANGO_DOMAIN_FACT_TYPES: &[&str] = &[
    "domain_concept_candidate",
    "lifecycle_candidate",
    "business_rule_candidate",
    "flow_step",
    "concept_relationship",
    "boundary_risk",
    "side_effect",
    "open_question",
    "pending_decision",
    "glossary_term_candidate",
];

/// Source classes for a fact. Increment 1 emits only programmatic and heuristic facts.
pub const DJANGO_DOMAIN_FACT_ORIGINS: &[&str] = &["programmatic", "heuristic", "llm", "human"];

/// Evidence basis for a fact. Confirmed is reserved for a later review loop.
pub const DJANGO_DOMAIN_FACT_BASES: &[&str] = &["observed", "inferred", "confirmed"];

/// Review statuses for facts. Increment 1 extraction emits only proposed facts.
pub const DJANGO_DOMAIN_FACT_STATUSES: &[&str] = &["proposed", "confirmed", "rejected", "stale"];

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

/// Candidate domain knowledge extracted from one evidence pack.
///
/// `subject` remains a backward-compatible alias for `technical_subject`. `pack_id`
/// indexes the evidence pack in merged JSONL output, while `primary_concept` and
/// `field_concept` are candidate business concepts that must remain reviewable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoDomainFact {
    pub schema_version: u32,
    pub id: String,
    pub pack_id: String,
    pub statement: String,
    #[serde(rename = "type")]
    pub fact_type: String,
    /// Backward-compatible alias for the technical subject, such as Reservation.status.
    pub subject: String,
    pub technical_subject: String,
    pub primary_concept: Option<String>,
    pub field_concept: Option<String>,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub confidence: String,
    pub origin: String,
    pub basis: String,
    pub status: String,
    pub rationale: String,
}

#[derive(Debug, Clone)]
struct DomainFactContext {
    pack_id: String,
    technical_subject: String,
    primary_concept: Option<String>,
    field_concept: Option<String>,
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
    extract_django_domain_facts_from_packs(std::slice::from_ref(pack))
}

/// Extracts a single fact set from one or more evidence packs.
///
/// Merge policy: facts are deduplicated by `(pack_id, technical_subject, type, statement)`.
/// Duplicate facts merge and deduplicate evidence, keep the strongest confidence, and keep
/// the most review-sensitive basis/origin when values differ.
pub fn extract_django_domain_facts_from_packs(
    packs: &[DjangoEvidencePack],
) -> Vec<DjangoDomainFact> {
    let facts = packs
        .iter()
        .flat_map(extract_django_domain_facts_for_pack)
        .collect::<Vec<_>>();

    finalize_domain_facts(facts)
}

fn extract_django_domain_facts_for_pack(pack: &DjangoEvidencePack) -> Vec<DjangoDomainFact> {
    let model_name = domain_model_name(pack);
    let field_name = domain_field_name(pack);
    let context = domain_fact_context(pack, &model_name, &field_name);
    let mut facts = Vec::new();

    push_domain_fact(
        &mut facts,
        &context,
        "domain_concept_candidate",
        format!("{model_name} appears to be a domain concept in this behavior slice."),
        model_evidence(pack),
        pack.lifecycle.confidence.clone(),
        "programmatic",
        "observed",
        "The evidence pack found a Django model as the primary subject of the inspected behavior.",
    );

    if pack.lifecycle.field.is_some()
        && (!pack.lifecycle.states.is_empty() || !pack.mutation_sites.is_empty())
    {
        push_domain_fact(
            &mut facts,
            &context,
            "lifecycle_candidate",
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
            &context,
            "lifecycle_candidate",
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
            &context,
            "concept_relationship",
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
                &context,
                "flow_step",
                format!("{model_name}.{field_name} is set to `{state}` through {channel}."),
                mutation_evidence.clone(),
                site.confidence.clone(),
                "programmatic",
                "observed",
                "A mutation site writes a detected lifecycle state to the subject field.",
            );
            push_domain_fact(
                &mut facts,
                &context,
                "business_rule_candidate",
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
                &context,
                "flow_step",
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
                &context,
                "side_effect",
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
            &context,
            "boundary_risk",
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
            &context,
            fact_type,
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
            &context,
            "open_question",
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
                &context,
                "pending_decision",
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
        &context,
        "glossary_term_candidate",
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
            &context,
            "glossary_term_candidate",
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

    facts
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
    context: &DomainFactContext,
    fact_type: &str,
    statement: String,
    evidence: Vec<DjangoSubjectEvidence>,
    confidence: String,
    origin: &str,
    basis: &str,
    rationale: &str,
) {
    if evidence.is_empty() || weak_generic_fact(fact_type, &confidence, evidence.len()) {
        return;
    }

    facts.push(DjangoDomainFact {
        schema_version: DJANGO_DOMAIN_FACT_SCHEMA_VERSION,
        id: String::new(),
        pack_id: context.pack_id.clone(),
        statement,
        fact_type: fact_type.to_owned(),
        subject: context.technical_subject.clone(),
        technical_subject: context.technical_subject.clone(),
        primary_concept: context.primary_concept.clone(),
        field_concept: context.field_concept.clone(),
        evidence,
        confidence,
        origin: origin.to_owned(),
        basis: basis.to_owned(),
        status: "proposed".to_owned(),
        rationale: rationale.to_owned(),
    });
}

fn finalize_domain_facts(facts: Vec<DjangoDomainFact>) -> Vec<DjangoDomainFact> {
    let mut merged = BTreeMap::<(String, String, String, String), DjangoDomainFact>::new();

    for mut fact in facts {
        sort_and_dedup_evidence(&mut fact.evidence);
        let key = (
            fact.pack_id.clone(),
            fact.technical_subject.clone(),
            fact.fact_type.clone(),
            fact.statement.clone(),
        );

        if let Some(existing) = merged.get_mut(&key) {
            existing.evidence.append(&mut fact.evidence);
            sort_and_dedup_evidence(&mut existing.evidence);
            existing.confidence = strongest_confidence(&existing.confidence, &fact.confidence);
            existing.origin = merged_origin(&existing.origin, &fact.origin);
            existing.basis = merged_basis(&existing.basis, &fact.basis);
        } else {
            merged.insert(key, fact);
        }
    }

    let mut facts = merged.into_values().collect::<Vec<_>>();
    facts.sort_by(|left, right| {
        left.pack_id
            .cmp(&right.pack_id)
            .then(left.fact_type.cmp(&right.fact_type))
            .then(left.technical_subject.cmp(&right.technical_subject))
            .then(left.statement.cmp(&right.statement))
    });

    let mut counters = BTreeMap::<String, usize>::new();
    for (index, fact) in facts.iter_mut().enumerate() {
        let counter = counters.entry(fact.pack_id.clone()).or_default();
        *counter += 1;
        fact.id = format!("{}-fact-{counter:03}", fact.pack_id);

        if fact.pack_id.is_empty() {
            fact.id = format!("domain-fact-{:03}", index + 1);
        }
    }

    facts
}

fn domain_fact_context(
    pack: &DjangoEvidencePack,
    model_name: &str,
    field_name: &str,
) -> DomainFactContext {
    DomainFactContext {
        pack_id: django_subject_slug(&pack.subject),
        technical_subject: pack.subject.clone(),
        primary_concept: pack.lifecycle.model.as_ref().map(|_| model_name.to_owned()),
        field_concept: pack.lifecycle.field.as_ref().map(|_| field_name.to_owned()),
    }
}

fn weak_generic_fact(fact_type: &str, confidence: &str, evidence_count: usize) -> bool {
    matches!(
        fact_type,
        "domain_concept_candidate" | "glossary_term_candidate" | "lifecycle_candidate"
    ) && confidence == "low"
        && evidence_count <= 1
}

fn sort_and_dedup_evidence(evidence: &mut Vec<DjangoSubjectEvidence>) {
    evidence.sort();
    evidence.dedup();
}

fn strongest_confidence(left: &str, right: &str) -> String {
    if confidence_rank(right) > confidence_rank(left) {
        right.to_owned()
    } else {
        left.to_owned()
    }
}

fn confidence_rank(confidence: &str) -> usize {
    match confidence {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn merged_origin(left: &str, right: &str) -> String {
    if left == right {
        left.to_owned()
    } else if left == "human" || right == "human" {
        "human".to_owned()
    } else if left == "llm" || right == "llm" {
        "llm".to_owned()
    } else if left == "heuristic" || right == "heuristic" {
        "heuristic".to_owned()
    } else {
        "programmatic".to_owned()
    }
}

fn merged_basis(left: &str, right: &str) -> String {
    if left == right {
        left.to_owned()
    } else if left == "confirmed" || right == "confirmed" {
        "confirmed".to_owned()
    } else if left == "inferred" || right == "inferred" {
        "inferred".to_owned()
    } else {
        "observed".to_owned()
    }
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
    evidence.extend(
        pack.mutation_sites
            .iter()
            .take(4)
            .map(|site| DjangoSubjectEvidence {
                path: site.path.clone(),
                line: site.line,
                detail: format!("{} mutation in {}", site.kind, site.container_name),
            }),
    );
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
    let value_tokens = domain_tokens(value);

    states
        .iter()
        .find(|state| {
            let normalized_state = domain_token(&state.value);
            !normalized_state.is_empty()
                && value_tokens.iter().any(|token| token == &normalized_state)
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

fn domain_tokens(value: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    let collapsed = domain_token(value);

    if !collapsed.is_empty() {
        tokens.insert(collapsed);
    }

    for token in value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(domain_token)
        .filter(|token| !token.is_empty())
    {
        tokens.insert(token);
    }

    tokens
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::python::subject::DjangoGuidanceSubjectSlice;

    #[test]
    fn merges_domain_facts_from_multiple_packs_with_pack_scoped_ids() {
        let reservation = minimal_pack(
            "Reservation.status",
            "Reservation",
            "status",
            &["pending", "confirmed"],
            vec![mutation_site(
                "webhook",
                "payments/webhooks.py",
                12,
                "payment_webhook",
                Some("Reservation.CONFIRMED"),
            )],
        );
        let payment = minimal_pack(
            "Payment.status",
            "Payment",
            "status",
            &["pending", "captured"],
            vec![mutation_site(
                "model_method",
                "payments/models.py",
                20,
                "Payment.capture",
                Some("Payment.CAPTURED"),
            )],
        );

        let facts = extract_django_domain_facts_from_packs(&[reservation, payment]);

        assert!(facts.iter().any(|fact| {
            fact.pack_id == "reservation-status"
                && fact.technical_subject == "Reservation.status"
                && fact.primary_concept.as_deref() == Some("Reservation")
                && fact.field_concept.as_deref() == Some("status")
        }));
        assert!(facts.iter().any(|fact| {
            fact.pack_id == "payment-status"
                && fact.id.starts_with("payment-status-fact-")
                && fact.statement.contains("Payment.status")
        }));
    }

    #[test]
    fn deduplicates_same_pack_facts_and_merges_evidence() {
        let first = minimal_pack(
            "Reservation.status",
            "Reservation",
            "status",
            &["confirmed"],
            vec![mutation_site(
                "webhook",
                "payments/webhooks.py",
                12,
                "payment_webhook",
                Some("Reservation.CONFIRMED"),
            )],
        );
        let duplicate = minimal_pack(
            "Reservation.status",
            "Reservation",
            "status",
            &["confirmed"],
            vec![mutation_site(
                "webhook",
                "payments/alternate_webhooks.py",
                18,
                "alternate_payment_webhook",
                Some("Reservation.CONFIRMED"),
            )],
        );

        let facts = extract_django_domain_facts_from_packs(&[first, duplicate]);
        let domain_concepts = facts
            .iter()
            .filter(|fact| fact.fact_type == "domain_concept_candidate")
            .collect::<Vec<_>>();

        assert_eq!(domain_concepts.len(), 1);
        assert_eq!(domain_concepts[0].pack_id, "reservation-status");
    }

    #[test]
    fn matches_mutation_states_by_exact_token_not_substring() {
        let pack = minimal_pack(
            "Trip.status",
            "Trip",
            "status",
            &["active", "inactive"],
            vec![mutation_site(
                "task",
                "trips/tasks.py",
                30,
                "deactivate_trips",
                Some("TripStatus.INACTIVE"),
            )],
        );

        let facts = extract_django_domain_facts(&pack);

        assert!(facts.iter().any(|fact| {
            fact.fact_type == "flow_step" && fact.statement.contains("`inactive`")
        }));
        assert!(!facts.iter().any(|fact| {
            matches!(
                fact.fact_type.as_str(),
                "flow_step" | "business_rule_candidate"
            ) && fact.statement.contains("`active`")
                && !fact.statement.contains("`inactive`")
        }));
    }

    fn minimal_pack(
        subject: &str,
        model_name: &str,
        field_name: &str,
        states: &[&str],
        mutation_sites: Vec<DjangoMutationSite>,
    ) -> DjangoEvidencePack {
        DjangoEvidencePack {
            schema_version: DJANGO_EVIDENCE_PACK_SCHEMA_VERSION,
            subject: subject.to_owned(),
            lifecycle: DjangoEvidenceLifecycle {
                model: Some(DjangoSubjectModel {
                    name: model_name.to_owned(),
                    qualified_name: model_name.to_owned(),
                    python_qualified_name: model_name.to_owned(),
                    path: format!("{}/models.py", model_name.to_ascii_lowercase()),
                    line: 10,
                    evidence: format!("class {model_name}(models.Model):"),
                    confidence: "high".to_owned(),
                }),
                field: Some(field_name.to_owned()),
                field_type: Some("CharField".to_owned()),
                states: states
                    .iter()
                    .enumerate()
                    .map(|(index, state)| DjangoSubjectState {
                        value: (*state).to_owned(),
                        path: format!("{}/models.py", model_name.to_ascii_lowercase()),
                        line: 20 + index,
                        evidence: format!("{state:?}"),
                        confidence: "high".to_owned(),
                    })
                    .collect(),
                confidence: "high".to_owned(),
            },
            relationship_map: DjangoEvidenceRelationshipMap {
                related_models: Vec::new(),
                related_components: Vec::new(),
                behavior_paths: Vec::new(),
                confidence: "medium".to_owned(),
            },
            mutation_sites,
            behavior_paths: Vec::new(),
            risk_signals: Vec::new(),
            open_questions: Vec::new(),
            reading_path: Vec::new(),
            related_tests: Vec::new(),
            coupling_signals: Vec::new(),
            evidence: Vec::new(),
            analysis_basis: DjangoGuidanceBasis {
                scope: "test pack".to_owned(),
                data_sources: Vec::new(),
                subject_slice: DjangoGuidanceSubjectSlice {
                    model_found: true,
                    lifecycle_candidate_found: true,
                    related_components: 0,
                    mutation_sites: 0,
                    behavior_paths: 0,
                    related_tests: 0,
                    evidence_items: 0,
                },
                caveats: Vec::new(),
            },
            confidence: DjangoEvidenceConfidence {
                subject: "high".to_owned(),
                behavior: "high".to_owned(),
                guidance: "medium".to_owned(),
                overall: "medium".to_owned(),
            },
        }
    }

    fn mutation_site(
        container_kind: &str,
        path: &str,
        line: usize,
        container_name: &str,
        value: Option<&str>,
    ) -> DjangoMutationSite {
        DjangoMutationSite {
            kind: "direct_assignment".to_owned(),
            container_kind: container_kind.to_owned(),
            container_name: container_name.to_owned(),
            path: path.to_owned(),
            line,
            evidence: format!("status = {}", value.unwrap_or("<unknown>")),
            mutation: format!("status = {}", value.unwrap_or("<unknown>")),
            value: value.map(str::to_owned),
            confidence: "high".to_owned(),
        }
    }
}

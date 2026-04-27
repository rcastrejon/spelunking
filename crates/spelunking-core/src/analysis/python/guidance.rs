use super::subject::{
    DjangoBehaviorPath, DjangoBehaviorReport, DjangoMutationSite, DjangoSubjectError,
    DjangoSubjectEvidence, DjangoSubjectReport, SubjectParts, inspect_django_behavior,
    inspect_django_subject, normalize_token,
};
use crate::{graph::relative_path_identifier, parsing::ParsedPythonModule};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjangoGuidanceReport {
    pub subject: String,
    pub analysis_basis: DjangoGuidanceBasis,
    pub risks: Vec<DjangoRiskSignal>,
    pub open_questions: Vec<DjangoOpenQuestion>,
    pub reading_path: Vec<DjangoReadingPathEntry>,
    pub related_tests: Vec<DjangoRelatedTest>,
    pub coupling_signals: Vec<DjangoCouplingSignal>,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjangoGuidanceBasis {
    pub scope: String,
    pub data_sources: Vec<String>,
    pub subject_slice: DjangoGuidanceSubjectSlice,
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjangoGuidanceSubjectSlice {
    pub model_found: bool,
    pub lifecycle_candidate_found: bool,
    pub related_components: usize,
    pub mutation_sites: usize,
    pub behavior_paths: usize,
    pub related_tests: usize,
    pub evidence_items: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjangoRiskSignal {
    pub title: String,
    pub severity: String,
    pub description: String,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjangoOpenQuestion {
    pub question: String,
    pub reason: String,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjangoReadingPathEntry {
    pub priority: usize,
    pub path: String,
    pub line: usize,
    pub reason: String,
    pub evidence: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjangoRelatedTest {
    pub path: String,
    pub line: usize,
    pub reason: String,
    pub evidence: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DjangoCouplingSignal {
    pub kind: String,
    pub description: String,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub confidence: String,
}

pub fn inspect_django_guidance(
    root: impl AsRef<Path>,
    modules: &[ParsedPythonModule],
    subject: &str,
) -> Result<DjangoGuidanceReport, DjangoSubjectError> {
    let root = root.as_ref();
    let subject_report = inspect_django_subject(root, modules, subject)?;
    let behavior_report = inspect_django_behavior(root, modules, subject)?;
    let parts = SubjectParts::parse(subject)?;

    let related_tests =
        collect_related_tests(root, modules, &subject_report, &behavior_report, &parts);
    let analysis_basis = build_analysis_basis(&subject_report, &behavior_report, &related_tests);
    let coupling_signals = build_coupling_signals(&subject_report, &behavior_report);
    let risks = build_risk_signals(
        &subject_report,
        &behavior_report,
        &related_tests,
        &coupling_signals,
    );
    let open_questions =
        build_open_questions(&subject_report, &behavior_report, &risks, &coupling_signals);
    let reading_path = build_reading_path(&subject_report, &behavior_report, &related_tests);
    let evidence = guidance_evidence(
        &risks,
        &open_questions,
        &reading_path,
        &related_tests,
        &coupling_signals,
    );
    let confidence = guidance_confidence(&behavior_report, &risks, &reading_path);

    Ok(DjangoGuidanceReport {
        subject: subject_report.subject,
        analysis_basis,
        risks,
        open_questions,
        reading_path,
        related_tests,
        coupling_signals,
        evidence,
        confidence,
    })
}

fn build_analysis_basis(
    subject_report: &DjangoSubjectReport,
    behavior_report: &DjangoBehaviorReport,
    related_tests: &[DjangoRelatedTest],
) -> DjangoGuidanceBasis {
    let evidence_items = subject_report.evidence.len() + behavior_report.evidence.len();

    DjangoGuidanceBasis {
        scope: "subject-focused behavioral slice".to_owned(),
        data_sources: vec![
            "inspect_django_subject structural radiography".to_owned(),
            "inspect_django_behavior mutation sites and behavior paths".to_owned(),
            "test-file scan for model, field, state, and mutation-method mentions".to_owned(),
        ],
        subject_slice: DjangoGuidanceSubjectSlice {
            model_found: subject_report.model.is_some(),
            lifecycle_candidate_found: subject_report.lifecycle_candidate.is_some(),
            related_components: subject_report.related_components.len(),
            mutation_sites: behavior_report.mutation_sites.len(),
            behavior_paths: behavior_report.behavior_paths.len(),
            related_tests: related_tests.len(),
            evidence_items,
        },
        caveats: vec![
            "This guidance is not built from a literal GraphExport subgraph; it uses the subject and behavior analyzers as the subject slice.".to_owned(),
            "Risk and coupling signals are heuristic and depend on detected mutation sites, behavior paths, path naming, and app-area inference.".to_owned(),
            "Absence of a risk or test match should be treated as absence of evidence, not proof that the behavior is safe or untested.".to_owned(),
        ],
    }
}

fn build_risk_signals(
    subject_report: &DjangoSubjectReport,
    behavior_report: &DjangoBehaviorReport,
    related_tests: &[DjangoRelatedTest],
    coupling_signals: &[DjangoCouplingSignal],
) -> Vec<DjangoRiskSignal> {
    let mut risks = Vec::new();
    let layers = mutation_layers(&behavior_report.mutation_sites);
    let apps = mutation_apps(subject_report, &behavior_report.mutation_sites);

    if layers.len() >= 2 {
        risks.push(DjangoRiskSignal {
            title: "Distributed lifecycle ownership".to_owned(),
            severity: if layers.len() >= 4 { "high" } else { "medium" }.to_owned(),
            description: format!(
                "{} is modified from {}.",
                behavior_report.subject,
                human_join(&layers.iter().cloned().collect::<Vec<_>>())
            ),
            evidence: mutation_evidence_by_layers(&behavior_report.mutation_sites, &layers, 4),
            confidence: "high".to_owned(),
        });
    }

    if apps.len() >= 2 {
        risks.push(DjangoRiskSignal {
            title: "Cross-app mutation".to_owned(),
            severity: "high".to_owned(),
            description: format!(
                "{} is modified from multiple app areas: {}.",
                behavior_report.subject,
                human_join(&apps.iter().cloned().collect::<Vec<_>>())
            ),
            evidence: mutation_evidence_by_apps(&behavior_report.mutation_sites, &apps, 4),
            confidence: "medium".to_owned(),
        });
    }

    if behavior_report.mutation_sites.iter().any(|site| {
        matches!(
            site.container_kind.as_str(),
            "task" | "signal_handler" | "webhook" | "management_command"
        )
    }) {
        risks.push(DjangoRiskSignal {
            title: "Out-of-request state changes".to_owned(),
            severity: "high".to_owned(),
            description: format!(
                "{} changes outside the normal API request path through async, signal, webhook, or command code.",
                behavior_report.subject
            ),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .filter(|site| {
                    matches!(
                        site.container_kind.as_str(),
                        "task" | "signal_handler" | "webhook" | "management_command"
                    )
                })
                .take(4)
                .map(mutation_evidence)
                .collect(),
            confidence: "high".to_owned(),
        });
    }

    if behavior_report
        .mutation_sites
        .iter()
        .any(|site| site.container_kind == "admin_action")
    {
        risks.push(DjangoRiskSignal {
            title: "Admin bypass risk".to_owned(),
            severity: "medium".to_owned(),
            description: format!(
                "Admin code mutates {} directly and may bypass API serializer or service validation.",
                behavior_report.subject
            ),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .filter(|site| site.container_kind == "admin_action")
                .take(3)
                .map(mutation_evidence)
                .collect(),
            confidence: "high".to_owned(),
        });
    }

    if behavior_report
        .mutation_sites
        .iter()
        .any(|site| matches!(site.container_kind.as_str(), "serializer" | "form"))
    {
        risks.push(DjangoRiskSignal {
            title: "Domain logic in validation layer".to_owned(),
            severity: "medium".to_owned(),
            description: format!(
                "Serializer or form code mutates {}, so validation and domain transitions may be coupled.",
                behavior_report.subject
            ),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .filter(|site| matches!(site.container_kind.as_str(), "serializer" | "form"))
                .take(3)
                .map(mutation_evidence)
                .collect(),
            confidence: "high".to_owned(),
        });
    }

    if has_direct_queryset_mutation(&behavior_report.mutation_sites) {
        risks.push(DjangoRiskSignal {
            title: "Bulk transition bypass".to_owned(),
            severity: "medium".to_owned(),
            description: format!(
                "Queryset or bulk updates touch {}, which can skip model methods and per-instance validation.",
                behavior_report.subject
            ),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .filter(|site| matches!(site.kind.as_str(), "queryset_update" | "bulk_update"))
                .take(4)
                .map(mutation_evidence)
                .collect(),
            confidence: "high".to_owned(),
        });
    }

    if layers.len() >= 3 || apps.len() >= 2 {
        risks.push(DjangoRiskSignal {
            title: "No obvious single lifecycle owner".to_owned(),
            severity: "medium".to_owned(),
            description: format!(
                "{} has multiple mutation owners; check whether one module is meant to own the transition rules.",
                behavior_report.subject
            ),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .take(5)
                .map(mutation_evidence)
                .collect(),
            confidence: "medium".to_owned(),
        });
    }

    if related_tests.is_empty() && !behavior_report.mutation_sites.is_empty() {
        risks.push(DjangoRiskSignal {
            title: "No nearby tests detected".to_owned(),
            severity: "medium".to_owned(),
            description: format!(
                "No test file clearly mentions {}, so expected lifecycle behavior may be hard to verify.",
                behavior_report.subject
            ),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .take(3)
                .map(mutation_evidence)
                .collect(),
            confidence: "medium".to_owned(),
        });
    } else if tests_are_dispersed(related_tests) {
        risks.push(DjangoRiskSignal {
            title: "Dispersed tests".to_owned(),
            severity: "low".to_owned(),
            description: format!(
                "Tests mentioning {} appear in multiple areas, so expectations may be split across workflows.",
                behavior_report.subject
            ),
            evidence: related_tests
                .iter()
                .take(4)
                .map(test_evidence)
                .collect(),
            confidence: "medium".to_owned(),
        });
    }

    for signal in coupling_signals {
        if signal.kind == "cross_app_mutation" {
            risks.push(DjangoRiskSignal {
                title: "App boundary mutation".to_owned(),
                severity: "high".to_owned(),
                description: signal.description.clone(),
                evidence: signal.evidence.clone(),
                confidence: signal.confidence.clone(),
            });
        }
    }

    risks.sort_by(|left, right| {
        severity_rank(&right.severity)
            .cmp(&severity_rank(&left.severity))
            .then(left.title.cmp(&right.title))
    });
    risks.dedup_by(|left, right| left.title == right.title);
    risks
}

fn build_open_questions(
    subject_report: &DjangoSubjectReport,
    behavior_report: &DjangoBehaviorReport,
    risks: &[DjangoRiskSignal],
    coupling_signals: &[DjangoCouplingSignal],
) -> Vec<DjangoOpenQuestion> {
    let mut questions = Vec::new();

    if risks
        .iter()
        .any(|risk| risk.title == "Distributed lifecycle ownership")
    {
        questions.push(DjangoOpenQuestion {
            question: format!(
                "Which module should own valid transitions for {}?",
                behavior_report.subject
            ),
            reason: "Mutations are spread across multiple layers.".to_owned(),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .take(4)
                .map(mutation_evidence)
                .collect(),
            confidence: "high".to_owned(),
        });
    }

    for signal in coupling_signals
        .iter()
        .filter(|signal| signal.kind == "cross_app_mutation")
        .take(2)
    {
        questions.push(DjangoOpenQuestion {
            question: format!(
                "Should this external app transition {}, or should it call a domain API on the owning app?",
                behavior_report.subject
            ),
            reason: signal.description.clone(),
            evidence: signal.evidence.clone(),
            confidence: signal.confidence.clone(),
        });
    }

    if behavior_report
        .mutation_sites
        .iter()
        .any(|site| site.container_kind == "admin_action")
    {
        questions.push(DjangoOpenQuestion {
            question: format!(
                "Do admin changes to {} need the same validation as API changes?",
                behavior_report.subject
            ),
            reason:
                "Admin actions can update rows without going through serializers or view logic."
                    .to_owned(),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .filter(|site| site.container_kind == "admin_action")
                .take(2)
                .map(mutation_evidence)
                .collect(),
            confidence: "high".to_owned(),
        });
    }

    if behavior_report.mutation_sites.iter().any(|site| {
        matches!(
            site.container_kind.as_str(),
            "task" | "signal_handler" | "webhook"
        )
    }) {
        questions.push(DjangoOpenQuestion {
            question: format!(
                "What side effects should happen when {} changes outside request flow?",
                behavior_report.subject
            ),
            reason: "Async, signal, or webhook paths can run without the API context.".to_owned(),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .filter(|site| {
                    matches!(
                        site.container_kind.as_str(),
                        "task" | "signal_handler" | "webhook"
                    )
                })
                .take(3)
                .map(mutation_evidence)
                .collect(),
            confidence: "high".to_owned(),
        });
    }

    if has_direct_queryset_mutation(&behavior_report.mutation_sites) {
        questions.push(DjangoOpenQuestion {
            question: format!(
                "Can bulk updates to {} skip required model methods, signals, or audit hooks?",
                behavior_report.subject
            ),
            reason: "Queryset and bulk updates do not behave like per-instance transitions."
                .to_owned(),
            evidence: behavior_report
                .mutation_sites
                .iter()
                .filter(|site| matches!(site.kind.as_str(), "queryset_update" | "bulk_update"))
                .take(3)
                .map(mutation_evidence)
                .collect(),
            confidence: "high".to_owned(),
        });
    }

    if let Some(candidate) = &subject_report.lifecycle_candidate
        && !candidate.states.is_empty()
    {
        questions.push(DjangoOpenQuestion {
            question: format!(
                "Which transitions among {} are valid for {}?",
                human_join(
                    &candidate
                        .states
                        .iter()
                        .map(|state| state.value.clone())
                        .collect::<Vec<_>>()
                ),
                behavior_report.subject
            ),
            reason:
                "Detected lifecycle states need explicit transition rules before changing behavior."
                    .to_owned(),
            evidence: candidate
                .states
                .iter()
                .take(4)
                .map(|state| DjangoSubjectEvidence {
                    path: state.path.clone(),
                    line: state.line,
                    detail: format!("detected state {}", state.value),
                })
                .collect(),
            confidence: "medium".to_owned(),
        });
    }

    questions.sort_by(|left, right| left.question.cmp(&right.question));
    questions.dedup_by(|left, right| left.question == right.question);
    questions.into_iter().take(6).collect()
}

fn build_reading_path(
    subject_report: &DjangoSubjectReport,
    behavior_report: &DjangoBehaviorReport,
    related_tests: &[DjangoRelatedTest],
) -> Vec<DjangoReadingPathEntry> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();

    if let Some(model) = &subject_report.model {
        push_reading_entry(
            &mut entries,
            &mut seen,
            &model.path,
            model.line,
            "Understand the model fields, lifecycle constants, and model-level transitions.",
            &model.evidence,
            "high",
        );
    }

    for component in subject_report
        .related_components
        .iter()
        .filter(|component| component.kind == "url")
    {
        push_reading_entry(
            &mut entries,
            &mut seen,
            &component.path,
            component.line,
            "Start from the route to see the external entrypoint into this behavior.",
            &component.evidence,
            &component.confidence,
        );
    }

    for component in subject_report
        .related_components
        .iter()
        .filter(|component| component.kind == "view")
    {
        push_reading_entry(
            &mut entries,
            &mut seen,
            &component.path,
            component.line,
            "Read API orchestration and request-level branching before changing the lifecycle.",
            &component.evidence,
            &component.confidence,
        );
    }

    for component in subject_report
        .related_components
        .iter()
        .filter(|component| matches!(component.kind.as_str(), "serializer" | "form"))
    {
        push_reading_entry(
            &mut entries,
            &mut seen,
            &component.path,
            component.line,
            "Check validation and data shaping near this subject.",
            &component.evidence,
            &component.confidence,
        );
    }

    for site in prioritized_mutation_sites(&behavior_report.mutation_sites) {
        push_reading_entry(
            &mut entries,
            &mut seen,
            &site.path,
            site.line,
            &reading_reason_for_site(site),
            &site.evidence,
            &site.confidence,
        );
    }

    for test in related_tests.iter().take(3) {
        push_reading_entry(
            &mut entries,
            &mut seen,
            &test.path,
            test.line,
            "Review existing expectations for this lifecycle before editing production code.",
            &test.evidence,
            &test.confidence,
        );
    }

    for (index, entry) in entries.iter_mut().enumerate() {
        entry.priority = index + 1;
    }

    entries.into_iter().take(10).collect()
}

fn push_reading_entry(
    entries: &mut Vec<DjangoReadingPathEntry>,
    seen: &mut HashSet<String>,
    path: &str,
    line: usize,
    reason: &str,
    evidence: &str,
    confidence: &str,
) {
    if !seen.insert(path.to_owned()) {
        return;
    }

    entries.push(DjangoReadingPathEntry {
        priority: 0,
        path: path.to_owned(),
        line,
        reason: reason.to_owned(),
        evidence: evidence.to_owned(),
        confidence: confidence.to_owned(),
    });
}

fn collect_related_tests(
    root: &Path,
    modules: &[ParsedPythonModule],
    subject_report: &DjangoSubjectReport,
    behavior_report: &DjangoBehaviorReport,
    parts: &SubjectParts,
) -> Vec<DjangoRelatedTest> {
    let state_values = subject_report
        .lifecycle_candidate
        .as_ref()
        .map(|candidate| {
            candidate
                .states
                .iter()
                .map(|state| normalize_token(&state.value))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let method_names = behavior_report
        .mutation_sites
        .iter()
        .filter_map(|site| {
            site.container_name
                .rsplit_once('.')
                .map(|(_, name)| normalize_token(name))
        })
        .collect::<BTreeSet<_>>();
    let mut tests = Vec::new();

    for module in modules {
        let module_path = relative_path_identifier(root, &module.path);

        if !is_test_module_path(&module_path) {
            continue;
        }

        let source = module.source.to_ascii_lowercase();
        let normalized_source = normalize_token(&module.source);
        let model = parts.model_name.to_ascii_lowercase();
        let field = parts.field_name.to_ascii_lowercase();
        let mentions_model = source.contains(&model);
        let mentions_field = source.contains(&field);
        let mentions_state = state_values
            .iter()
            .any(|state| !state.is_empty() && normalized_source.contains(state));
        let mentions_method = method_names
            .iter()
            .any(|method| !method.is_empty() && normalized_source.contains(method));

        if !(mentions_model && (mentions_field || mentions_state || mentions_method)) {
            continue;
        }

        let (line, evidence) = first_relevant_test_line(
            module,
            &parts.model_name,
            &parts.field_name,
            &state_values,
            &method_names,
        );
        let reason = if mentions_field {
            format!("mentions {} and {}", parts.model_name, parts.field_name)
        } else if mentions_state {
            format!(
                "mentions {} and detected lifecycle states",
                parts.model_name
            )
        } else {
            format!("mentions {} and mutation method names", parts.model_name)
        };

        tests.push(DjangoRelatedTest {
            path: module_path,
            line,
            reason,
            evidence,
            confidence: if mentions_field { "high" } else { "medium" }.to_owned(),
        });
    }

    tests.sort_by(|left, right| {
        confidence_rank(&right.confidence)
            .cmp(&confidence_rank(&left.confidence))
            .then(left.path.cmp(&right.path))
    });
    tests.dedup_by(|left, right| left.path == right.path);
    tests.into_iter().take(8).collect()
}

fn first_relevant_test_line(
    module: &ParsedPythonModule,
    model_name: &str,
    field_name: &str,
    state_values: &[String],
    method_names: &BTreeSet<String>,
) -> (usize, String) {
    let model = model_name.to_ascii_lowercase();
    let field = field_name.to_ascii_lowercase();

    for (index, line) in module.source.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        let normalized = normalize_token(line);

        if lower.contains(&model)
            || lower.contains(&field)
            || state_values
                .iter()
                .any(|state| !state.is_empty() && normalized.contains(state))
            || method_names
                .iter()
                .any(|method| !method.is_empty() && normalized.contains(method))
        {
            return (index + 1, line.trim().to_owned());
        }
    }

    (1, String::new())
}

fn build_coupling_signals(
    subject_report: &DjangoSubjectReport,
    behavior_report: &DjangoBehaviorReport,
) -> Vec<DjangoCouplingSignal> {
    let mut signals = Vec::new();
    let model_app = subject_report
        .model
        .as_ref()
        .map(|model| app_area_from_path(&model.path));
    let layer_pairs = behavior_layer_pairs(&behavior_report.behavior_paths);

    if let Some(model_app) = model_app {
        let mut cross_app_sites = behavior_report
            .mutation_sites
            .iter()
            .filter(|site| {
                let app = app_area_from_path(&site.path);
                app != model_app && app != "unknown"
            })
            .collect::<Vec<_>>();
        cross_app_sites
            .sort_by(|left, right| left.path.cmp(&right.path).then(left.line.cmp(&right.line)));

        if !cross_app_sites.is_empty() {
            let apps = cross_app_sites
                .iter()
                .map(|site| app_area_from_path(&site.path))
                .collect::<BTreeSet<_>>();
            signals.push(DjangoCouplingSignal {
                kind: "cross_app_mutation".to_owned(),
                description: format!(
                    "{} is owned near app area '{}' but is mutated from {}.",
                    behavior_report.subject,
                    model_app,
                    human_join(&apps.into_iter().collect::<Vec<_>>())
                ),
                evidence: cross_app_sites
                    .into_iter()
                    .take(4)
                    .map(mutation_evidence)
                    .collect(),
                confidence: "medium".to_owned(),
            });
        }
    }

    if !layer_pairs.is_empty() {
        signals.push(DjangoCouplingSignal {
            kind: "layered_behavior_path".to_owned(),
            description: format!(
                "{} has behavior paths that cross layers such as {}.",
                behavior_report.subject,
                human_join(&layer_pairs.into_iter().take(4).collect::<Vec<_>>())
            ),
            evidence: behavior_report
                .behavior_paths
                .iter()
                .flat_map(|path| path.steps.iter().take(2))
                .take(5)
                .map(step_evidence)
                .collect(),
            confidence: "medium".to_owned(),
        });
    }

    signals
}

fn behavior_layer_pairs(behavior_paths: &[DjangoBehaviorPath]) -> BTreeSet<String> {
    let mut pairs = BTreeSet::new();

    for path in behavior_paths {
        let layers = path
            .steps
            .iter()
            .map(|step| step.kind.as_str())
            .filter(|kind| *kind != "subject")
            .collect::<Vec<_>>();

        for window in layers.windows(2) {
            if let [left, right] = window
                && left != right
            {
                pairs.insert(format!("{left} -> {right}"));
            }
        }
    }

    pairs
}

fn prioritized_mutation_sites(sites: &[DjangoMutationSite]) -> Vec<&DjangoMutationSite> {
    let mut sites = sites.iter().collect::<Vec<_>>();

    sites.sort_by(|left, right| {
        mutation_layer_priority(&left.container_kind)
            .cmp(&mutation_layer_priority(&right.container_kind))
            .then(left.path.cmp(&right.path))
            .then(left.line.cmp(&right.line))
    });

    sites
}

fn reading_reason_for_site(site: &DjangoMutationSite) -> String {
    match site.container_kind.as_str() {
        "model_method" => {
            "Understand the canonical model-level transition or persistence behavior."
        }
        "view" => {
            "Inspect request-driven mutation logic and API permissions around this transition."
        }
        "serializer" | "form" => "Inspect validation-layer mutations before changing domain rules.",
        "task" => "Inspect async mutation behavior and scheduling assumptions.",
        "signal_handler" => "Inspect implicit mutation behavior triggered by Django signals.",
        "webhook" => "Inspect external event-driven transitions and idempotency assumptions.",
        "admin_action" => "Inspect manual/admin transition paths that can bypass API validation.",
        "management_command" => "Inspect command-line maintenance paths that mutate this state.",
        _ => "Inspect this mutation site before changing the lifecycle.",
    }
    .to_owned()
}

fn mutation_layers(sites: &[DjangoMutationSite]) -> BTreeSet<String> {
    sites
        .iter()
        .map(|site| human_layer_name(&site.container_kind).to_owned())
        .collect()
}

fn mutation_apps(
    subject_report: &DjangoSubjectReport,
    sites: &[DjangoMutationSite],
) -> BTreeSet<String> {
    let mut apps = BTreeSet::new();

    if let Some(model) = &subject_report.model {
        apps.insert(app_area_from_path(&model.path));
    }

    for site in sites {
        apps.insert(app_area_from_path(&site.path));
    }

    apps.remove("unknown");
    apps
}

fn mutation_evidence_by_layers(
    sites: &[DjangoMutationSite],
    layers: &BTreeSet<String>,
    limit: usize,
) -> Vec<DjangoSubjectEvidence> {
    let mut evidence = Vec::new();
    let mut seen = HashSet::new();

    for layer in layers {
        if let Some(site) = sites
            .iter()
            .find(|site| human_layer_name(&site.container_kind) == layer)
            && seen.insert(site.container_kind.clone())
        {
            evidence.push(mutation_evidence(site));
        }

        if evidence.len() >= limit {
            break;
        }
    }

    evidence
}

fn mutation_evidence_by_apps(
    sites: &[DjangoMutationSite],
    apps: &BTreeSet<String>,
    limit: usize,
) -> Vec<DjangoSubjectEvidence> {
    let mut evidence = Vec::new();
    let mut seen = HashSet::new();

    for app in apps {
        if let Some(site) = sites
            .iter()
            .find(|site| app_area_from_path(&site.path) == *app)
            && seen.insert(app.clone())
        {
            evidence.push(mutation_evidence(site));
        }

        if evidence.len() >= limit {
            break;
        }
    }

    evidence
}

fn app_area_from_path(path: &str) -> String {
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if let Some((index, _)) = parts.iter().enumerate().find(|(_, part)| **part == "apps")
        && let Some(app) = parts.get(index + 1)
    {
        return (*app).to_owned();
    }

    for marker in [
        "models",
        "views",
        "serializers",
        "forms",
        "tasks",
        "signals",
        "admin",
        "webhooks",
        "management",
        "tests",
    ] {
        if let Some(index) = parts
            .iter()
            .position(|part| part.trim_end_matches(".py") == marker)
            && index > 0
        {
            return parts[index - 1].to_owned();
        }
    }

    parts.first().copied().unwrap_or("unknown").to_owned()
}

fn is_test_module_path(path: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.rsplit('/').next().is_some_and(|file_name| {
            file_name.starts_with("test_") || file_name.ends_with("_test.py")
        })
}

fn has_direct_queryset_mutation(sites: &[DjangoMutationSite]) -> bool {
    sites
        .iter()
        .any(|site| matches!(site.kind.as_str(), "queryset_update" | "bulk_update"))
}

fn tests_are_dispersed(tests: &[DjangoRelatedTest]) -> bool {
    tests
        .iter()
        .map(|test| app_area_from_path(&test.path))
        .collect::<BTreeSet<_>>()
        .len()
        >= 2
}

fn mutation_layer_priority(kind: &str) -> usize {
    match kind {
        "model_method" => 0,
        "view" => 1,
        "serializer" | "form" => 2,
        "task" => 3,
        "signal_handler" => 4,
        "webhook" => 5,
        "admin_action" => 6,
        "management_command" => 7,
        _ => 8,
    }
}

fn human_layer_name(kind: &str) -> &str {
    match kind {
        "model_method" => "model methods",
        "view" => "views",
        "serializer" => "serializers",
        "form" => "forms",
        "task" => "tasks",
        "signal_handler" => "signals",
        "webhook" => "webhooks",
        "admin_action" => "admin actions",
        "management_command" => "management commands",
        "async_function" => "async functions",
        _ => "functions",
    }
}

fn severity_rank(severity: &str) -> usize {
    match severity {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
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

fn mutation_evidence(site: &DjangoMutationSite) -> DjangoSubjectEvidence {
    DjangoSubjectEvidence {
        path: site.path.clone(),
        line: site.line,
        detail: format!("{} mutation in {}", site.kind, site.container_name),
    }
}

fn step_evidence(step: &super::subject::DjangoBehaviorStep) -> DjangoSubjectEvidence {
    DjangoSubjectEvidence {
        path: step.path.clone(),
        line: step.line,
        detail: format!("{} step {}", step.kind, step.name),
    }
}

fn test_evidence(test: &DjangoRelatedTest) -> DjangoSubjectEvidence {
    DjangoSubjectEvidence {
        path: test.path.clone(),
        line: test.line,
        detail: test.reason.clone(),
    }
}

fn guidance_evidence(
    risks: &[DjangoRiskSignal],
    questions: &[DjangoOpenQuestion],
    reading_path: &[DjangoReadingPathEntry],
    tests: &[DjangoRelatedTest],
    coupling_signals: &[DjangoCouplingSignal],
) -> Vec<DjangoSubjectEvidence> {
    let mut evidence = BTreeSet::new();

    for risk in risks {
        for item in &risk.evidence {
            evidence.insert(item.clone());
        }
    }

    for question in questions {
        for item in &question.evidence {
            evidence.insert(item.clone());
        }
    }

    for entry in reading_path {
        evidence.insert(DjangoSubjectEvidence {
            path: entry.path.clone(),
            line: entry.line,
            detail: format!("reading path: {}", entry.reason),
        });
    }

    for test in tests {
        evidence.insert(test_evidence(test));
    }

    for signal in coupling_signals {
        for item in &signal.evidence {
            evidence.insert(item.clone());
        }
    }

    evidence.into_iter().collect()
}

fn guidance_confidence(
    behavior_report: &DjangoBehaviorReport,
    risks: &[DjangoRiskSignal],
    reading_path: &[DjangoReadingPathEntry],
) -> String {
    if behavior_report.mutation_sites.is_empty() {
        return "low".to_owned();
    }

    if risks.iter().any(|risk| risk.confidence == "high") && reading_path.len() >= 3 {
        "high".to_owned()
    } else if !risks.is_empty() || reading_path.len() >= 2 {
        "medium".to_owned()
    } else {
        "low".to_owned()
    }
}

fn human_join(values: &[String]) -> String {
    match values {
        [] => "none".to_owned(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        many => {
            let mut values = many.to_vec();
            let last = values.pop().unwrap_or_default();
            format!("{}, and {last}", values.join(", "))
        }
    }
}

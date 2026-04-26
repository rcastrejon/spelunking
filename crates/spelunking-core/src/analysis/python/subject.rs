use super::imports::{ImportIndex, expr_dotted_name, python_module_path};
use crate::{graph::relative_path_identifier, parsing::ParsedPythonModule};
use rustpython_parser::ast::{
    self, Constant, Expr, Ranged, Stmt, StmtAsyncFunctionDef, StmtClassDef, StmtFunctionDef,
};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    error::Error,
    fmt,
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoSubjectReport {
    pub subject: String,
    pub model: Option<DjangoSubjectModel>,
    pub lifecycle_candidate: Option<DjangoLifecycleCandidate>,
    pub fields: Vec<DjangoSubjectField>,
    pub related_models: Vec<DjangoRelatedModel>,
    pub relevant_methods: Vec<DjangoRelevantMethod>,
    pub related_components: Vec<DjangoRelatedComponent>,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoSubjectModel {
    pub name: String,
    pub qualified_name: String,
    pub python_qualified_name: String,
    pub path: String,
    pub line: usize,
    pub evidence: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoLifecycleCandidate {
    pub field: String,
    pub field_type: String,
    pub states: Vec<DjangoSubjectState>,
    pub line: usize,
    pub evidence: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoSubjectState {
    pub value: String,
    pub path: String,
    pub line: usize,
    pub evidence: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoSubjectField {
    pub name: String,
    pub field_type: String,
    pub path: String,
    pub line: usize,
    pub evidence: String,
    pub is_subject: bool,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoRelatedModel {
    pub model: String,
    pub field: String,
    pub relationship: String,
    pub path: String,
    pub line: usize,
    pub evidence: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoRelevantMethod {
    pub name: String,
    pub path: String,
    pub line: usize,
    pub evidence: String,
    pub reason: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoRelatedComponent {
    pub kind: String,
    pub name: String,
    pub path: String,
    pub line: usize,
    pub evidence: String,
    pub reason: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct DjangoSubjectEvidence {
    pub path: String,
    pub line: usize,
    pub detail: String,
}

pub use super::artifacts::{
    DJANGO_EVIDENCE_PACK_SCHEMA_VERSION, DjangoArtifactBundle, DjangoEvidenceConfidence,
    DjangoEvidenceLifecycle, DjangoEvidencePack, DjangoEvidenceRelationshipMap,
    build_django_artifact_bundle, build_django_evidence_pack, django_subject_slug,
    render_django_evaluation_report, render_django_markdown_report,
};
pub use super::behavior::{
    DjangoBehaviorPath, DjangoBehaviorReport, DjangoBehaviorStep, DjangoMutationSite,
    inspect_django_behavior,
};
pub use super::guidance::{
    DjangoCouplingSignal, DjangoGuidanceBasis, DjangoGuidanceReport, DjangoGuidanceSubjectSlice,
    DjangoOpenQuestion, DjangoReadingPathEntry, DjangoRelatedTest, DjangoRiskSignal,
    inspect_django_guidance,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DjangoSubjectCandidate {
    pub subject: String,
    pub model: String,
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DjangoSubjectError {
    InvalidSubject(String),
    AmbiguousSubject {
        subject: String,
        candidates: Vec<DjangoSubjectCandidate>,
    },
}

impl fmt::Display for DjangoSubjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSubject(subject) => write!(
                f,
                "invalid subject '{subject}'. Expected a Django subject like Model.field or app.Model.field"
            ),
            Self::AmbiguousSubject {
                subject,
                candidates,
            } => {
                let suggestions = candidates
                    .iter()
                    .map(|candidate| {
                        format!(
                            "{} ({}:{})",
                            candidate.subject, candidate.path, candidate.line
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                write!(
                    f,
                    "ambiguous subject '{subject}'. Found multiple plausible models; use one of: {suggestions}"
                )
            }
        }
    }
}

impl Error for DjangoSubjectError {}

#[derive(Debug, Clone)]
pub(super) struct SubjectParts {
    pub(super) raw: String,
    pub(super) model_path: String,
    pub(super) model_name: String,
    pub(super) field_name: String,
    pub(super) app_hint: Option<String>,
}

#[derive(Clone)]
pub(super) struct DiscoveredClass<'a> {
    pub(super) module: &'a ParsedPythonModule,
    pub(super) module_path: String,
    pub(super) python_module: String,
    pub(super) import_index: ImportIndex,
    pub(super) class_def: &'a StmtClassDef,
    pub(super) qualified_name: String,
    pub(super) python_qualified_name: String,
}

#[derive(Clone)]
pub(super) struct ModelCandidate<'a> {
    pub(super) class: DiscoveredClass<'a>,
    pub(super) score: usize,
    pub(super) confidence: &'static str,
}

#[derive(Debug, Clone)]
struct DiscoveredField {
    name: String,
    field_type: String,
    resolved_field_type: String,
    path: String,
    line: usize,
    evidence: String,
    relationship: Option<DiscoveredRelationship>,
}

#[derive(Debug, Clone)]
struct DiscoveredRelationship {
    kind: String,
    target: String,
}

pub fn inspect_django_subject(
    root: impl AsRef<Path>,
    modules: &[ParsedPythonModule],
    subject: &str,
) -> Result<DjangoSubjectReport, DjangoSubjectError> {
    let root = root.as_ref();
    let parts = SubjectParts::parse(subject)?;
    let classes = discover_classes(root, modules);
    let candidates = find_model_candidates(&classes, &parts);
    let candidate = resolve_model_candidate(&parts, candidates)?;

    let Some(candidate) = candidate else {
        return Ok(DjangoSubjectReport {
            subject: parts.raw,
            model: None,
            lifecycle_candidate: None,
            fields: Vec::new(),
            related_models: Vec::new(),
            relevant_methods: Vec::new(),
            related_components: Vec::new(),
            evidence: Vec::new(),
            confidence: "low".to_owned(),
        });
    };

    Ok(build_report(parts, &candidate, &classes))
}

impl SubjectParts {
    pub(super) fn parse(subject: &str) -> Result<Self, DjangoSubjectError> {
        let trimmed = subject.trim();
        let Some((model_path, field_name)) = trimmed.rsplit_once('.') else {
            return Err(DjangoSubjectError::InvalidSubject(subject.to_owned()));
        };

        if model_path.is_empty() || field_name.is_empty() {
            return Err(DjangoSubjectError::InvalidSubject(subject.to_owned()));
        }

        let model_name = model_path
            .rsplit('.')
            .next()
            .filter(|model_name| !model_name.is_empty())
            .ok_or_else(|| DjangoSubjectError::InvalidSubject(subject.to_owned()))?
            .to_owned();
        let app_hint = model_path
            .rsplit_once('.')
            .map(|(prefix, _)| prefix)
            .filter(|prefix| !prefix.is_empty())
            .map(ToOwned::to_owned);

        Ok(Self {
            raw: trimmed.to_owned(),
            model_path: model_path.to_owned(),
            model_name,
            field_name: field_name.to_owned(),
            app_hint,
        })
    }
}

pub(super) fn discover_classes<'a>(
    root: &Path,
    modules: &'a [ParsedPythonModule],
) -> Vec<DiscoveredClass<'a>> {
    let mut classes = Vec::new();

    for module in modules {
        let module_path = relative_path_identifier(root, &module.path);

        if is_test_module_path(&module_path) {
            continue;
        }

        let python_module = python_module_path(&module_path);
        let import_index = ImportIndex::from_suite(&module.ast, &python_module);

        collect_classes(
            &mut classes,
            module,
            &module_path,
            &python_module,
            &import_index,
            &module.ast,
            &mut Vec::new(),
        );
    }

    classes
}

fn collect_classes<'a>(
    classes: &mut Vec<DiscoveredClass<'a>>,
    module: &'a ParsedPythonModule,
    module_path: &str,
    python_module: &str,
    import_index: &ImportIndex,
    suite: &'a ast::Suite,
    class_stack: &mut Vec<String>,
) {
    for statement in suite {
        let Stmt::ClassDef(class_def) = statement else {
            continue;
        };

        class_stack.push(class_def.name.to_string());

        let qualified_name = class_stack.join(".");
        let python_qualified_name = format!("{python_module}.{qualified_name}");
        classes.push(DiscoveredClass {
            module,
            module_path: module_path.to_owned(),
            python_module: python_module.to_owned(),
            import_index: import_index.clone(),
            class_def,
            qualified_name,
            python_qualified_name,
        });

        collect_classes(
            classes,
            module,
            module_path,
            python_module,
            import_index,
            &class_def.body,
            class_stack,
        );

        class_stack.pop();
    }
}

pub(super) fn find_model_candidates<'a>(
    classes: &'a [DiscoveredClass<'a>],
    parts: &SubjectParts,
) -> Vec<ModelCandidate<'a>> {
    let mut candidates = classes
        .iter()
        .filter(|class| class_name(&class.qualified_name) == parts.model_name)
        .filter(|class| {
            parts
                .app_hint
                .as_ref()
                .is_none_or(|app_hint| class_matches_app_hint(class, app_hint))
        })
        .map(|class| {
            let mut score = 1;

            if is_django_model_like(class) {
                score += 50;
            }

            if class.module_path.ends_with("models.py") || class.module_path.contains("/models/") {
                score += 20;
            }

            if let Some(app_hint) = &parts.app_hint
                && class_matches_app_hint(class, app_hint)
            {
                score += 20;
            }

            if class.python_qualified_name.ends_with(&parts.model_path) {
                score += 10;
            }

            let confidence = if score >= 50 {
                "high"
            } else if score >= 20 {
                "medium"
            } else {
                "low"
            };

            ModelCandidate {
                class: class.clone(),
                score,
                confidence,
            }
        })
        .filter(|candidate| candidate.score >= 50)
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then(left.class.module_path.cmp(&right.class.module_path))
            .then(left.class.qualified_name.cmp(&right.class.qualified_name))
    });
    candidates
}

pub(super) fn resolve_model_candidate<'a>(
    parts: &SubjectParts,
    candidates: Vec<ModelCandidate<'a>>,
) -> Result<Option<ModelCandidate<'a>>, DjangoSubjectError> {
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.into_iter().next()),
        _ => Err(DjangoSubjectError::AmbiguousSubject {
            subject: parts.raw.clone(),
            candidates: candidates
                .iter()
                .map(|candidate| DjangoSubjectCandidate {
                    subject: candidate_subject(&candidate.class, &parts.field_name),
                    model: candidate.class.qualified_name.clone(),
                    path: candidate.class.module_path.clone(),
                    line: line_for_node(candidate.class.module, candidate.class.class_def),
                })
                .collect(),
        }),
    }
}

fn build_report(
    parts: SubjectParts,
    candidate: &ModelCandidate<'_>,
    classes: &[DiscoveredClass<'_>],
) -> DjangoSubjectReport {
    let class = &candidate.class;
    let model_line = line_for_node(class.module, class.class_def);
    let model = DjangoSubjectModel {
        name: parts.model_name.clone(),
        qualified_name: class.qualified_name.clone(),
        python_qualified_name: class.python_qualified_name.clone(),
        path: class.module_path.clone(),
        line: model_line,
        evidence: source_line(class.module, model_line),
        confidence: candidate.confidence.to_owned(),
    };

    let constants = class_constants(&class.class_def.body);
    let choice_sequences = class_choice_sequences(class.module, &class.class_def.body, &constants);
    let nested_choice_states = merged_choice_states(
        nested_choice_states(class.module, &class.module_path, &class.class_def.body),
        global_choice_states(classes),
    );
    let discovered_fields = collect_model_fields(class.module, class, &constants);
    let fields = discovered_fields
        .iter()
        .map(|field| DjangoSubjectField {
            name: field.name.clone(),
            field_type: field.field_type.clone(),
            path: field.path.clone(),
            line: field.line,
            evidence: field.evidence.clone(),
            is_subject: field.name == parts.field_name,
            confidence: "high".to_owned(),
        })
        .collect::<Vec<_>>();
    let related_models = discovered_fields
        .iter()
        .filter_map(|field| {
            let relationship = field.relationship.as_ref()?;

            Some(DjangoRelatedModel {
                model: relationship.target.clone(),
                field: field.name.clone(),
                relationship: relationship.kind.clone(),
                path: field.path.clone(),
                line: field.line,
                evidence: field.evidence.clone(),
                confidence: "high".to_owned(),
            })
        })
        .collect::<Vec<_>>();
    let subject_field = discovered_fields
        .iter()
        .find(|field| field.name == parts.field_name);
    let lifecycle_candidate = subject_field.map(|field| {
        let states = field_states(
            class.module,
            class,
            &parts.field_name,
            &constants,
            &choice_sequences,
            &nested_choice_states,
        );

        DjangoLifecycleCandidate {
            field: field.name.clone(),
            field_type: field.field_type.clone(),
            states,
            line: field.line,
            evidence: field.evidence.clone(),
            confidence: lifecycle_confidence(field),
        }
    });
    let relevant_methods = collect_relevant_methods(
        class.module,
        &class.module_path,
        &class.class_def.body,
        &parts.field_name,
        lifecycle_candidate
            .as_ref()
            .map(|candidate| candidate.states.as_slice())
            .unwrap_or(&[]),
    );
    let related_components = collect_related_components(classes, &parts, class);
    let evidence = report_evidence(
        &model,
        lifecycle_candidate.as_ref(),
        &fields,
        &related_models,
        &relevant_methods,
        &related_components,
    );
    let confidence = report_confidence(lifecycle_candidate.as_ref(), &related_components);

    DjangoSubjectReport {
        subject: parts.raw,
        model: Some(model),
        lifecycle_candidate,
        fields,
        related_models,
        relevant_methods,
        related_components,
        evidence,
        confidence,
    }
}

fn collect_model_fields(
    module: &ParsedPythonModule,
    class: &DiscoveredClass<'_>,
    constants: &HashMap<String, String>,
) -> Vec<DiscoveredField> {
    let mut fields = Vec::new();

    for statement in &class.class_def.body {
        let line = line_for_node(module, statement);
        let evidence = source_line(module, line);

        match statement {
            Stmt::Assign(assign) => {
                if let Some((field_type, resolved_field_type, relationship)) =
                    django_field_from_expr(&assign.value, &class.import_index, constants)
                {
                    for name in assign.targets.iter().filter_map(assignment_name) {
                        fields.push(DiscoveredField {
                            name,
                            field_type: field_type.clone(),
                            resolved_field_type: resolved_field_type.clone(),
                            path: class.module_path.clone(),
                            line,
                            evidence: evidence.clone(),
                            relationship: relationship.clone(),
                        });
                    }
                }
            }
            Stmt::AnnAssign(assign) => {
                if let Some(value) = &assign.value
                    && let Some((field_type, resolved_field_type, relationship)) =
                        django_field_from_expr(value, &class.import_index, constants)
                    && let Some(name) = assignment_name(&assign.target)
                {
                    fields.push(DiscoveredField {
                        name,
                        field_type,
                        resolved_field_type,
                        path: class.module_path.clone(),
                        line,
                        evidence,
                        relationship,
                    });
                }
            }
            _ => {}
        }
    }

    fields.sort_by_key(|field| field.line);
    fields
}

fn django_field_from_expr(
    expr: &Expr,
    import_index: &ImportIndex,
    constants: &HashMap<String, String>,
) -> Option<(String, String, Option<DiscoveredRelationship>)> {
    let Expr::Call(call) = expr else {
        return None;
    };

    let field_type = expr_dotted_name(&call.func)?;
    let resolved_field_type = import_index.resolve(&field_type);

    if !is_django_field_type(&resolved_field_type) {
        return None;
    }

    let relationship = relationship_kind(&resolved_field_type).and_then(|kind| {
        let target = call
            .args
            .first()
            .or_else(|| keyword_arg_expr(&call.keywords, "to"))
            .and_then(|expr| model_reference_value(expr, import_index, constants))?;

        Some(DiscoveredRelationship {
            kind: kind.to_owned(),
            target: display_model_name(&target),
        })
    });

    Some((
        class_name(&resolved_field_type).to_owned(),
        resolved_field_type,
        relationship,
    ))
}

fn is_django_field_type(value: &str) -> bool {
    value.starts_with("django.db.models.")
        && (value.ends_with("Field")
            || matches!(
                value,
                "django.db.models.ForeignKey"
                    | "django.db.models.OneToOneField"
                    | "django.db.models.ManyToManyField"
            ))
}

fn relationship_kind(value: &str) -> Option<&'static str> {
    match value {
        "django.db.models.ForeignKey" => Some("foreign_key"),
        "django.db.models.OneToOneField" => Some("one_to_one"),
        "django.db.models.ManyToManyField" => Some("many_to_many"),
        _ => None,
    }
}

fn field_states(
    module: &ParsedPythonModule,
    class: &DiscoveredClass<'_>,
    field_name: &str,
    constants: &HashMap<String, String>,
    choice_sequences: &HashMap<String, Vec<DjangoSubjectState>>,
    nested_choice_states: &HashMap<String, Vec<DjangoSubjectState>>,
) -> Vec<DjangoSubjectState> {
    let Some(statement) = class
        .class_def
        .body
        .iter()
        .find(|statement| assignment_targets_field(statement, field_name))
    else {
        return Vec::new();
    };
    let Some(value) = assignment_value(statement) else {
        return Vec::new();
    };
    let Expr::Call(call) = value else {
        return Vec::new();
    };

    let mut states = Vec::new();

    if let Some(choices_expr) = keyword_arg_expr(&call.keywords, "choices") {
        states.extend(states_from_choices_expr(
            module,
            choices_expr,
            constants,
            choice_sequences,
            nested_choice_states,
        ));
    }

    if let Some(default_expr) = keyword_arg_expr(&call.keywords, "default")
        && let Some(value) = state_value(default_expr, constants, nested_choice_states)
    {
        let line = line_for_node(module, default_expr);

        states.push(DjangoSubjectState {
            value,
            path: class.module_path.clone(),
            line,
            evidence: source_line(module, line),
            confidence: "medium".to_owned(),
        });
    }

    for state in &mut states {
        if state.path.is_empty() {
            state.path = class.module_path.clone();
        }
    }

    dedupe_states(states)
}

fn states_from_choices_expr(
    module: &ParsedPythonModule,
    expr: &Expr,
    constants: &HashMap<String, String>,
    choice_sequences: &HashMap<String, Vec<DjangoSubjectState>>,
    nested_choice_states: &HashMap<String, Vec<DjangoSubjectState>>,
) -> Vec<DjangoSubjectState> {
    match expr {
        Expr::List(list) => states_from_choice_items(module, &list.elts, constants),
        Expr::Tuple(tuple) => states_from_choice_items(module, &tuple.elts, constants),
        Expr::Set(set) => states_from_choice_items(module, &set.elts, constants),
        Expr::Name(name) => choice_sequences
            .get(name.id.as_str())
            .cloned()
            .unwrap_or_default(),
        Expr::Attribute(attribute) if attribute.attr.as_str() == "choices" => {
            let Some(choice_class) = expr_dotted_name(&attribute.value) else {
                return Vec::new();
            };

            nested_choice_states
                .get(class_name(&choice_class))
                .cloned()
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn states_from_choice_items(
    module: &ParsedPythonModule,
    items: &[Expr],
    constants: &HashMap<String, String>,
) -> Vec<DjangoSubjectState> {
    items
        .iter()
        .filter_map(|item| {
            let value_expr = match item {
                Expr::Tuple(tuple) => tuple.elts.first()?,
                Expr::List(list) => list.elts.first()?,
                _ => item,
            };
            let value = state_value(value_expr, constants, &HashMap::new())?;
            let line = line_for_node(module, item);

            Some(DjangoSubjectState {
                value,
                path: String::new(),
                line,
                evidence: source_line(module, line),
                confidence: "high".to_owned(),
            })
        })
        .collect()
}

fn global_choice_states(
    classes: &[DiscoveredClass<'_>],
) -> HashMap<String, Vec<DjangoSubjectState>> {
    let mut choices = HashMap::new();

    for class in classes {
        if !class_is_choices_class(class) {
            continue;
        }

        let states = choice_states_from_class(class);

        if !states.is_empty() {
            choices.insert(class_name(&class.qualified_name).to_owned(), states);
        }
    }

    choices
}

fn merged_choice_states(
    mut primary: HashMap<String, Vec<DjangoSubjectState>>,
    secondary: HashMap<String, Vec<DjangoSubjectState>>,
) -> HashMap<String, Vec<DjangoSubjectState>> {
    for (name, states) in secondary {
        primary.entry(name).or_insert(states);
    }

    primary
}

fn nested_choice_states(
    module: &ParsedPythonModule,
    module_path: &str,
    suite: &ast::Suite,
) -> HashMap<String, Vec<DjangoSubjectState>> {
    let mut choices = HashMap::new();

    for statement in suite {
        let Stmt::ClassDef(class_def) = statement else {
            continue;
        };

        if !class_def.bases.iter().any(is_choices_base) {
            continue;
        }

        let constants = class_constants(&class_def.body);
        let states = choice_states_from_suite(module, module_path, &class_def.body, &constants);

        if !states.is_empty() {
            choices.insert(class_def.name.to_string(), states);
        }
    }

    choices
}

fn class_is_choices_class(class: &DiscoveredClass<'_>) -> bool {
    class.class_def.bases.iter().any(|base| {
        let resolved = resolved_base_name(base, &class.import_index);
        is_choices_base_name(&resolved)
    })
}

fn is_choices_base(expr: &Expr) -> bool {
    expr_dotted_name(expr).is_some_and(|name| is_choices_base_name(&name))
}

fn is_choices_base_name(name: &str) -> bool {
    matches!(
        class_name(name),
        "TextChoices" | "IntegerChoices" | "Choices"
    )
}

fn choice_states_from_class(class: &DiscoveredClass<'_>) -> Vec<DjangoSubjectState> {
    let constants = class_constants(&class.class_def.body);

    choice_states_from_suite(
        class.module,
        &class.module_path,
        &class.class_def.body,
        &constants,
    )
}

fn choice_states_from_suite(
    module: &ParsedPythonModule,
    module_path: &str,
    suite: &ast::Suite,
    constants: &HashMap<String, String>,
) -> Vec<DjangoSubjectState> {
    suite
        .iter()
        .filter_map(|statement| {
            assignment_target_name(statement)?;
            let value = assignment_value(statement)?;
            let value = match value {
                Expr::Tuple(tuple) => tuple
                    .elts
                    .first()
                    .and_then(|expr| state_value(expr, constants, &HashMap::new()))?,
                _ => state_value(value, constants, &HashMap::new())?,
            };
            let line = line_for_node(module, statement);

            Some(DjangoSubjectState {
                value,
                path: module_path.to_owned(),
                line,
                evidence: source_line(module, line),
                confidence: "high".to_owned(),
            })
        })
        .collect()
}

fn class_choice_sequences(
    module: &ParsedPythonModule,
    suite: &ast::Suite,
    constants: &HashMap<String, String>,
) -> HashMap<String, Vec<DjangoSubjectState>> {
    let mut sequences = HashMap::new();

    for statement in suite {
        let Some(name) = assignment_target_name(statement) else {
            continue;
        };
        let Some(value) = assignment_value(statement) else {
            continue;
        };

        let states = match value {
            Expr::List(list) => states_from_choice_items(module, &list.elts, constants),
            Expr::Tuple(tuple) => states_from_choice_items(module, &tuple.elts, constants),
            _ => Vec::new(),
        };

        if !states.is_empty() {
            sequences.insert(name, states);
        }
    }

    sequences
}

fn dedupe_states(states: Vec<DjangoSubjectState>) -> Vec<DjangoSubjectState> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();

    for state in states {
        if seen.insert(state.value.clone()) {
            deduped.push(state);
        }
    }

    deduped.sort_by(|left, right| left.value.cmp(&right.value));
    deduped
}

fn collect_relevant_methods(
    module: &ParsedPythonModule,
    module_path: &str,
    suite: &ast::Suite,
    field_name: &str,
    states: &[DjangoSubjectState],
) -> Vec<DjangoRelevantMethod> {
    let state_tokens = states
        .iter()
        .map(|state| normalize_token(&state.value))
        .collect::<HashSet<_>>();
    let mut methods = Vec::new();

    for statement in suite {
        match statement {
            Stmt::FunctionDef(function_def) => {
                if let Some(method) =
                    method_if_relevant(module, module_path, function_def, field_name, &state_tokens)
                {
                    methods.push(method);
                }
            }
            Stmt::AsyncFunctionDef(function_def) => {
                if let Some(method) = async_method_if_relevant(
                    module,
                    module_path,
                    function_def,
                    field_name,
                    &state_tokens,
                ) {
                    methods.push(method);
                }
            }
            _ => {}
        }
    }

    methods.sort_by_key(|method| method.line);
    methods
}

fn method_if_relevant(
    module: &ParsedPythonModule,
    module_path: &str,
    function_def: &StmtFunctionDef,
    field_name: &str,
    state_tokens: &HashSet<String>,
) -> Option<DjangoRelevantMethod> {
    let (reason, confidence) = method_relevance(
        &function_def.name,
        &function_def.body,
        field_name,
        state_tokens,
    )?;
    let line = line_for_node(module, function_def);

    Some(DjangoRelevantMethod {
        name: function_def.name.to_string(),
        path: module_path.to_owned(),
        line,
        evidence: source_line(module, line),
        reason,
        confidence,
    })
}

fn async_method_if_relevant(
    module: &ParsedPythonModule,
    module_path: &str,
    function_def: &StmtAsyncFunctionDef,
    field_name: &str,
    state_tokens: &HashSet<String>,
) -> Option<DjangoRelevantMethod> {
    let (reason, confidence) = method_relevance(
        &function_def.name,
        &function_def.body,
        field_name,
        state_tokens,
    )?;
    let line = line_for_node(module, function_def);

    Some(DjangoRelevantMethod {
        name: function_def.name.to_string(),
        path: module_path.to_owned(),
        line,
        evidence: source_line(module, line),
        reason,
        confidence,
    })
}

fn method_relevance(
    name: &str,
    body: &ast::Suite,
    field_name: &str,
    state_tokens: &HashSet<String>,
) -> Option<(String, String)> {
    if suite_references_field(body, field_name) {
        return Some((format!("references `{field_name}`"), "high".to_owned()));
    }

    let normalized_name = normalize_token(name);
    if state_tokens
        .iter()
        .any(|state| !state.is_empty() && normalized_name.contains(state))
    {
        return Some((
            "method name matches a detected state".to_owned(),
            "medium".to_owned(),
        ));
    }

    None
}

fn collect_related_components(
    classes: &[DiscoveredClass<'_>],
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
) -> Vec<DjangoRelatedComponent> {
    let mut components = Vec::new();
    let mut serializer_names = HashSet::new();
    let mut view_names = HashSet::new();

    for class in classes {
        if class.python_qualified_name == model_class.python_qualified_name {
            continue;
        }

        if let Some(component) = serializer_or_form_component(class, parts) {
            serializer_names.insert(class_name(&class.qualified_name).to_owned());
            components.push(component);
        }
    }

    for class in classes {
        if class.python_qualified_name == model_class.python_qualified_name {
            continue;
        }

        if let Some(component) = view_component(class, parts, &serializer_names) {
            view_names.insert(class_name(&class.qualified_name).to_owned());
            components.push(component);
        }
    }

    components.extend(url_components(classes, &view_names));
    components.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.name.cmp(&right.name))
    });
    components.dedup_by(|left, right| {
        left.kind == right.kind
            && left.name == right.name
            && left.path == right.path
            && left.line == right.line
    });
    components
}

pub(super) fn serializer_or_form_component(
    class: &DiscoveredClass<'_>,
    parts: &SubjectParts,
) -> Option<DjangoRelatedComponent> {
    let kind = if class_is_serializer(class) {
        "serializer"
    } else if class_is_form(class) {
        "form"
    } else {
        return None;
    };

    let meta_model = meta_model_reference(&class.class_def.body, &class.import_index);
    let (reason, confidence) = if meta_model
        .as_deref()
        .is_some_and(|model| model_matches_subject(model, parts))
    {
        (format!("Meta.model = {}", parts.model_name), "high")
    } else if suite_references_model(&class.class_def.body, &parts.model_name) {
        (format!("references {}", parts.model_name), "medium")
    } else {
        return None;
    };
    let line = line_for_node(class.module, class.class_def);

    Some(DjangoRelatedComponent {
        kind: kind.to_owned(),
        name: class.qualified_name.clone(),
        path: class.module_path.clone(),
        line,
        evidence: source_line(class.module, line),
        reason,
        confidence: confidence.to_owned(),
    })
}

pub(super) fn view_component(
    class: &DiscoveredClass<'_>,
    parts: &SubjectParts,
    serializer_names: &HashSet<String>,
) -> Option<DjangoRelatedComponent> {
    if !class_is_view(class) {
        return None;
    }

    let (reason, confidence) = if suite_references_model(&class.class_def.body, &parts.model_name) {
        (format!("references {}", parts.model_name), "medium")
    } else if suite_references_any_name(&class.class_def.body, serializer_names) {
        ("references a related serializer".to_owned(), "medium")
    } else {
        return None;
    };
    let line = line_for_node(class.module, class.class_def);

    Some(DjangoRelatedComponent {
        kind: "view".to_owned(),
        name: class.qualified_name.clone(),
        path: class.module_path.clone(),
        line,
        evidence: source_line(class.module, line),
        reason,
        confidence: confidence.to_owned(),
    })
}

fn url_components(
    classes: &[DiscoveredClass<'_>],
    view_names: &HashSet<String>,
) -> Vec<DjangoRelatedComponent> {
    if view_names.is_empty() {
        return Vec::new();
    }

    let mut seen_modules = HashSet::new();
    let mut components = Vec::new();

    for class in classes {
        if !seen_modules.insert(class.module_path.clone())
            || !class.module_path.ends_with("urls.py")
        {
            continue;
        }

        for statement in &class.module.ast {
            collect_url_components_from_stmt(
                &mut components,
                class.module,
                &class.module_path,
                statement,
                view_names,
            );
        }
    }

    components
}

fn collect_url_components_from_stmt(
    components: &mut Vec<DjangoRelatedComponent>,
    module: &ParsedPythonModule,
    module_path: &str,
    statement: &Stmt,
    view_names: &HashSet<String>,
) {
    match statement {
        Stmt::Assign(assign) => collect_url_components_from_expr(
            components,
            module,
            module_path,
            &assign.value,
            view_names,
        ),
        Stmt::Expr(expr) => collect_url_components_from_expr(
            components,
            module,
            module_path,
            &expr.value,
            view_names,
        ),
        _ => {}
    }
}

fn collect_url_components_from_expr(
    components: &mut Vec<DjangoRelatedComponent>,
    module: &ParsedPythonModule,
    module_path: &str,
    expr: &Expr,
    view_names: &HashSet<String>,
) {
    match expr {
        Expr::Call(call) => {
            if expr_dotted_name(&call.func).is_some_and(|name| route_function_name(&name)) {
                let target = call.args.get(1).and_then(route_target_name);

                if target
                    .as_deref()
                    .is_some_and(|target| view_names.contains(class_name(target)))
                {
                    let route = call
                        .args
                        .first()
                        .and_then(string_constant)
                        .unwrap_or_else(|| "<dynamic route>".to_owned());
                    let line = line_for_node(module, expr);

                    components.push(DjangoRelatedComponent {
                        kind: "url".to_owned(),
                        name: route,
                        path: module_path.to_owned(),
                        line,
                        evidence: source_line(module, line),
                        reason: format!("routes to {}", target.unwrap_or_default()),
                        confidence: "medium".to_owned(),
                    });
                }
            }

            for arg in &call.args {
                collect_url_components_from_expr(components, module, module_path, arg, view_names);
            }
        }
        Expr::List(list) => {
            for item in &list.elts {
                collect_url_components_from_expr(components, module, module_path, item, view_names);
            }
        }
        Expr::Tuple(tuple) => {
            for item in &tuple.elts {
                collect_url_components_from_expr(components, module, module_path, item, view_names);
            }
        }
        _ => {}
    }
}

pub(super) fn route_function_name(name: &str) -> bool {
    matches!(class_name(name), "path" | "re_path" | "url")
}

pub(super) fn route_target_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Call(call) => expr_dotted_name(&call.func)
            .filter(|name| class_name(name) == "as_view")
            .and_then(|_| {
                expr_dotted_name(&call.func)
                    .and_then(|name| name.rsplit_once('.').map(|(prefix, _)| prefix.to_owned()))
            }),
        _ => expr_dotted_name(expr),
    }
}

fn class_is_serializer(class: &DiscoveredClass<'_>) -> bool {
    class.qualified_name.ends_with("Serializer")
        || class
            .class_def
            .bases
            .iter()
            .any(|base| resolved_base_name(base, &class.import_index).contains("Serializer"))
}

fn class_is_form(class: &DiscoveredClass<'_>) -> bool {
    class.qualified_name.ends_with("Form")
        || class
            .class_def
            .bases
            .iter()
            .any(|base| resolved_base_name(base, &class.import_index).ends_with("Form"))
}

pub(super) fn class_is_view(class: &DiscoveredClass<'_>) -> bool {
    class.qualified_name.ends_with("View")
        || class.qualified_name.ends_with("ViewSet")
        || class.class_def.bases.iter().any(|base| {
            let name = resolved_base_name(base, &class.import_index);
            name.ends_with("View") || name.ends_with("ViewSet") || name.ends_with("APIView")
        })
}

pub(super) fn resolved_base_name(expr: &Expr, import_index: &ImportIndex) -> String {
    expr_dotted_name(expr)
        .map(|name| import_index.resolve(&name))
        .unwrap_or_default()
}

fn is_django_model_like(class: &DiscoveredClass<'_>) -> bool {
    class.class_def.bases.iter().any(|base| {
        let name = resolved_base_name(base, &class.import_index);

        name == "django.db.models.Model"
            || name.ends_with(".Model")
            || class_name(&name).ends_with("Model")
    })
}

fn class_matches_app_hint(class: &DiscoveredClass<'_>, app_hint: &str) -> bool {
    let path_hint = app_hint.replace('.', "/");
    let app_hint_label = class_name(app_hint);

    class.python_module == app_hint
        || class.python_module.starts_with(&format!("{app_hint}."))
        || class.module_path == path_hint
        || class.module_path.starts_with(&format!("{path_hint}/"))
        || app_label(class).as_deref() == Some(app_hint_label)
}

fn candidate_subject(class: &DiscoveredClass<'_>, field_name: &str) -> String {
    format!(
        "{}.{}.{}",
        app_label(class).unwrap_or_else(|| class.python_module.clone()),
        class_name(&class.qualified_name),
        field_name
    )
}

fn app_label(class: &DiscoveredClass<'_>) -> Option<String> {
    app_label_from_module_path(&class.module_path)
}

fn app_label_from_module_path(module_path: &str) -> Option<String> {
    let parts = module_path.split('/').collect::<Vec<_>>();

    if let Some(apps_index) = parts.iter().position(|part| *part == "apps")
        && let Some(label) = parts.get(apps_index + 1)
        && !label.is_empty()
    {
        return Some((*label).to_owned());
    }

    if let Some(models_index) = parts
        .iter()
        .position(|part| *part == "models.py" || *part == "models")
        && models_index > 0
        && let Some(label) = parts.get(models_index - 1)
        && !label.is_empty()
    {
        return Some((*label).to_owned());
    }

    parts
        .first()
        .filter(|part| !part.is_empty())
        .map(|part| (*part).to_owned())
}

fn meta_model_reference(suite: &ast::Suite, import_index: &ImportIndex) -> Option<String> {
    suite.iter().find_map(|statement| {
        let Stmt::ClassDef(class_def) = statement else {
            return None;
        };

        if class_def.name.as_str() != "Meta" {
            return None;
        }

        class_def.body.iter().find_map(|statement| {
            let value = assignment_value(statement)?;

            if assignment_targets_field(statement, "model") {
                model_reference_value(value, import_index, &HashMap::new())
            } else {
                None
            }
        })
    })
}

fn model_reference_value(
    expr: &Expr,
    import_index: &ImportIndex,
    constants: &HashMap<String, String>,
) -> Option<String> {
    match expr {
        Expr::Constant(_) => string_constant(expr),
        Expr::Name(name) => constants
            .get(name.id.as_str())
            .cloned()
            .or_else(|| Some(import_index.resolve(name.id.as_str()))),
        _ => expr_dotted_name(expr).map(|name| import_index.resolve(&name)),
    }
}

fn model_matches_subject(model: &str, parts: &SubjectParts) -> bool {
    model == parts.model_path
        || model == parts.model_name
        || class_name(model) == parts.model_name
        || model.ends_with(&format!(".{}", parts.model_name))
}

fn display_model_name(value: &str) -> String {
    class_name(value).to_owned()
}

fn class_constants(suite: &ast::Suite) -> HashMap<String, String> {
    let mut constants = HashMap::new();

    for statement in suite {
        let Some(name) = assignment_target_name(statement) else {
            continue;
        };
        let Some(value) = assignment_value(statement).and_then(string_constant) else {
            continue;
        };

        constants.insert(name, value);
    }

    constants
}

fn assignment_target_name(statement: &Stmt) -> Option<String> {
    match statement {
        Stmt::Assign(assign) => assign.targets.iter().find_map(assignment_name),
        Stmt::AnnAssign(assign) => assignment_name(&assign.target),
        _ => None,
    }
}

fn assignment_targets_field(statement: &Stmt, field_name: &str) -> bool {
    match statement {
        Stmt::Assign(assign) => assign
            .targets
            .iter()
            .any(|target| assignment_name(target).as_deref() == Some(field_name)),
        Stmt::AnnAssign(assign) => assignment_name(&assign.target).as_deref() == Some(field_name),
        _ => false,
    }
}

fn assignment_value(statement: &Stmt) -> Option<&Expr> {
    match statement {
        Stmt::Assign(assign) => Some(&assign.value),
        Stmt::AnnAssign(assign) => assign.value.as_deref(),
        _ => None,
    }
}

fn assignment_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        _ => None,
    }
}

pub(super) fn keyword_arg_expr<'a>(keywords: &'a [ast::Keyword], name: &str) -> Option<&'a Expr> {
    keywords
        .iter()
        .find(|keyword| keyword.arg.as_ref().is_some_and(|arg| arg.as_str() == name))
        .map(|keyword| &keyword.value)
}

pub(super) fn string_constant(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Constant(constant) => match &constant.value {
            Constant::Str(value) => Some(value.clone()),
            Constant::Int(value) => Some(value.to_string()),
            Constant::Bool(value) => Some(value.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn state_value(
    expr: &Expr,
    constants: &HashMap<String, String>,
    nested_choice_states: &HashMap<String, Vec<DjangoSubjectState>>,
) -> Option<String> {
    match expr {
        Expr::Name(name) => constants.get(name.id.as_str()).cloned(),
        Expr::Attribute(attribute) => {
            let owner = expr_dotted_name(&attribute.value)?;
            nested_choice_states
                .get(class_name(&owner))
                .and_then(|states| {
                    states.iter().find(|state| {
                        normalize_token(&state.value) == normalize_token(attribute.attr.as_str())
                    })
                })
                .map(|state| state.value.clone())
        }
        _ => string_constant(expr),
    }
}

fn lifecycle_confidence(field: &DiscoveredField) -> String {
    if lifecycle_field_name(&field.name) {
        "high".to_owned()
    } else if likely_lifecycle_field_type(&field.resolved_field_type) {
        "medium".to_owned()
    } else {
        "low".to_owned()
    }
}

fn lifecycle_field_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "status" | "state" | "stage" | "phase"
    )
}

fn likely_lifecycle_field_type(field_type: &str) -> bool {
    matches!(
        class_name(field_type),
        "CharField" | "TextField" | "IntegerField" | "PositiveSmallIntegerField"
    )
}

fn suite_references_field(suite: &ast::Suite, field_name: &str) -> bool {
    suite
        .iter()
        .any(|statement| stmt_references_field(statement, field_name))
}

fn stmt_references_field(statement: &Stmt, field_name: &str) -> bool {
    match statement {
        Stmt::FunctionDef(function_def) => suite_references_field(&function_def.body, field_name),
        Stmt::AsyncFunctionDef(function_def) => {
            suite_references_field(&function_def.body, field_name)
        }
        Stmt::Return(statement) => statement
            .value
            .as_ref()
            .is_some_and(|value| expr_references_field(value, field_name)),
        Stmt::Assign(statement) => {
            statement
                .targets
                .iter()
                .any(|target| expr_references_field(target, field_name))
                || expr_references_field(&statement.value, field_name)
        }
        Stmt::AnnAssign(statement) => {
            expr_references_field(&statement.target, field_name)
                || statement
                    .value
                    .as_ref()
                    .is_some_and(|value| expr_references_field(value, field_name))
        }
        Stmt::AugAssign(statement) => {
            expr_references_field(&statement.target, field_name)
                || expr_references_field(&statement.value, field_name)
        }
        Stmt::Expr(statement) => expr_references_field(&statement.value, field_name),
        Stmt::If(statement) => {
            expr_references_field(&statement.test, field_name)
                || suite_references_field(&statement.body, field_name)
                || suite_references_field(&statement.orelse, field_name)
        }
        Stmt::For(statement) => {
            expr_references_field(&statement.target, field_name)
                || expr_references_field(&statement.iter, field_name)
                || suite_references_field(&statement.body, field_name)
                || suite_references_field(&statement.orelse, field_name)
        }
        Stmt::While(statement) => {
            expr_references_field(&statement.test, field_name)
                || suite_references_field(&statement.body, field_name)
                || suite_references_field(&statement.orelse, field_name)
        }
        Stmt::With(statement) => suite_references_field(&statement.body, field_name),
        Stmt::AsyncWith(statement) => suite_references_field(&statement.body, field_name),
        _ => false,
    }
}

fn expr_references_field(expr: &Expr, field_name: &str) -> bool {
    match expr {
        Expr::Name(name) => name.id.as_str() == field_name,
        Expr::Attribute(attribute) => {
            attribute.attr.as_str() == field_name
                || expr_references_field(&attribute.value, field_name)
        }
        Expr::Call(call) => {
            expr_references_field(&call.func, field_name)
                || call
                    .args
                    .iter()
                    .any(|arg| expr_references_field(arg, field_name))
                || call
                    .keywords
                    .iter()
                    .any(|keyword| expr_references_field(&keyword.value, field_name))
        }
        Expr::Compare(compare) => {
            expr_references_field(&compare.left, field_name)
                || compare
                    .comparators
                    .iter()
                    .any(|expr| expr_references_field(expr, field_name))
        }
        Expr::BoolOp(bool_op) => bool_op
            .values
            .iter()
            .any(|expr| expr_references_field(expr, field_name)),
        Expr::BinOp(bin_op) => {
            expr_references_field(&bin_op.left, field_name)
                || expr_references_field(&bin_op.right, field_name)
        }
        Expr::UnaryOp(unary_op) => expr_references_field(&unary_op.operand, field_name),
        Expr::IfExp(if_exp) => {
            expr_references_field(&if_exp.test, field_name)
                || expr_references_field(&if_exp.body, field_name)
                || expr_references_field(&if_exp.orelse, field_name)
        }
        Expr::List(list) => list
            .elts
            .iter()
            .any(|expr| expr_references_field(expr, field_name)),
        Expr::Tuple(tuple) => tuple
            .elts
            .iter()
            .any(|expr| expr_references_field(expr, field_name)),
        Expr::Dict(dict) => {
            dict.keys
                .iter()
                .flatten()
                .any(|expr| expr_references_field(expr, field_name))
                || dict
                    .values
                    .iter()
                    .any(|expr| expr_references_field(expr, field_name))
        }
        _ => false,
    }
}

pub(super) fn suite_references_model(suite: &ast::Suite, model_name: &str) -> bool {
    suite
        .iter()
        .any(|statement| stmt_references_model(statement, model_name))
}

fn stmt_references_model(statement: &Stmt, model_name: &str) -> bool {
    match statement {
        Stmt::ClassDef(class_def) => suite_references_model(&class_def.body, model_name),
        Stmt::FunctionDef(function_def) => suite_references_model(&function_def.body, model_name),
        Stmt::AsyncFunctionDef(function_def) => {
            suite_references_model(&function_def.body, model_name)
        }
        Stmt::Assign(statement) => {
            statement
                .targets
                .iter()
                .any(|target| expr_references_model(target, model_name))
                || expr_references_model(&statement.value, model_name)
        }
        Stmt::AnnAssign(statement) => {
            expr_references_model(&statement.target, model_name)
                || statement
                    .value
                    .as_ref()
                    .is_some_and(|value| expr_references_model(value, model_name))
        }
        Stmt::Return(statement) => statement
            .value
            .as_ref()
            .is_some_and(|value| expr_references_model(value, model_name)),
        Stmt::Expr(statement) => expr_references_model(&statement.value, model_name),
        Stmt::If(statement) => {
            expr_references_model(&statement.test, model_name)
                || suite_references_model(&statement.body, model_name)
                || suite_references_model(&statement.orelse, model_name)
        }
        _ => false,
    }
}

pub(super) fn expr_references_model(expr: &Expr, model_name: &str) -> bool {
    match expr {
        Expr::Name(name) => name.id.as_str() == model_name,
        Expr::Attribute(attribute) => {
            attribute.attr.as_str() == model_name
                || expr_references_model(&attribute.value, model_name)
        }
        Expr::Call(call) => {
            expr_references_model(&call.func, model_name)
                || call
                    .args
                    .iter()
                    .any(|arg| expr_references_model(arg, model_name))
                || call
                    .keywords
                    .iter()
                    .any(|keyword| expr_references_model(&keyword.value, model_name))
        }
        Expr::List(list) => list
            .elts
            .iter()
            .any(|expr| expr_references_model(expr, model_name)),
        Expr::Tuple(tuple) => tuple
            .elts
            .iter()
            .any(|expr| expr_references_model(expr, model_name)),
        Expr::Dict(dict) => {
            dict.keys
                .iter()
                .flatten()
                .any(|expr| expr_references_model(expr, model_name))
                || dict
                    .values
                    .iter()
                    .any(|expr| expr_references_model(expr, model_name))
        }
        _ => false,
    }
}

fn suite_references_any_name(suite: &ast::Suite, names: &HashSet<String>) -> bool {
    !names.is_empty()
        && suite
            .iter()
            .any(|statement| stmt_references_any_name(statement, names))
}

fn stmt_references_any_name(statement: &Stmt, names: &HashSet<String>) -> bool {
    match statement {
        Stmt::ClassDef(class_def) => suite_references_any_name(&class_def.body, names),
        Stmt::FunctionDef(function_def) => suite_references_any_name(&function_def.body, names),
        Stmt::AsyncFunctionDef(function_def) => {
            suite_references_any_name(&function_def.body, names)
        }
        Stmt::Assign(statement) => expr_references_any_name(&statement.value, names),
        Stmt::AnnAssign(statement) => statement
            .value
            .as_ref()
            .is_some_and(|value| expr_references_any_name(value, names)),
        Stmt::Expr(statement) => expr_references_any_name(&statement.value, names),
        _ => false,
    }
}

fn expr_references_any_name(expr: &Expr, names: &HashSet<String>) -> bool {
    match expr {
        Expr::Name(name) => names.contains(name.id.as_str()),
        Expr::Attribute(attribute) => {
            names.contains(attribute.attr.as_str())
                || expr_references_any_name(&attribute.value, names)
        }
        Expr::Call(call) => {
            expr_references_any_name(&call.func, names)
                || call
                    .args
                    .iter()
                    .any(|arg| expr_references_any_name(arg, names))
                || call
                    .keywords
                    .iter()
                    .any(|keyword| expr_references_any_name(&keyword.value, names))
        }
        Expr::List(list) => list
            .elts
            .iter()
            .any(|expr| expr_references_any_name(expr, names)),
        Expr::Tuple(tuple) => tuple
            .elts
            .iter()
            .any(|expr| expr_references_any_name(expr, names)),
        _ => false,
    }
}

fn report_evidence(
    model: &DjangoSubjectModel,
    lifecycle: Option<&DjangoLifecycleCandidate>,
    fields: &[DjangoSubjectField],
    related_models: &[DjangoRelatedModel],
    methods: &[DjangoRelevantMethod],
    components: &[DjangoRelatedComponent],
) -> Vec<DjangoSubjectEvidence> {
    let mut evidence = BTreeSet::new();

    evidence.insert(DjangoSubjectEvidence {
        path: model.path.clone(),
        line: model.line,
        detail: format!("{} model definition", model.name),
    });

    if let Some(lifecycle) = lifecycle {
        evidence.insert(DjangoSubjectEvidence {
            path: model.path.clone(),
            line: lifecycle.line,
            detail: format!("{} field definition", lifecycle.field),
        });
    }

    for field in fields.iter().filter(|field| field.is_subject) {
        evidence.insert(DjangoSubjectEvidence {
            path: field.path.clone(),
            line: field.line,
            detail: format!("subject field {}", field.name),
        });
    }

    for relationship in related_models {
        evidence.insert(DjangoSubjectEvidence {
            path: relationship.path.clone(),
            line: relationship.line,
            detail: format!(
                "{} relates to {} through {}",
                model.name, relationship.model, relationship.field
            ),
        });
    }

    for method in methods {
        evidence.insert(DjangoSubjectEvidence {
            path: method.path.clone(),
            line: method.line,
            detail: format!("relevant method {}", method.name),
        });
    }

    for component in components {
        evidence.insert(DjangoSubjectEvidence {
            path: component.path.clone(),
            line: component.line,
            detail: format!("related {} {}", component.kind, component.name),
        });
    }

    evidence.into_iter().collect()
}

fn report_confidence(
    lifecycle: Option<&DjangoLifecycleCandidate>,
    components: &[DjangoRelatedComponent],
) -> String {
    if lifecycle.is_some_and(|candidate| candidate.confidence == "high")
        && components
            .iter()
            .any(|component| component.confidence == "high")
    {
        "high".to_owned()
    } else if lifecycle.is_some() {
        "medium".to_owned()
    } else {
        "low".to_owned()
    }
}

fn is_test_module_path(module_path: &str) -> bool {
    module_path.starts_with("tests/")
        || module_path.contains("/tests/")
        || module_path.rsplit('/').next().is_some_and(|file_name| {
            file_name.starts_with("test_") || file_name.ends_with("_test.py")
        })
}

pub(super) fn line_for_node<T: Ranged>(module: &ParsedPythonModule, node: &T) -> usize {
    let offset = node.start().to_usize().min(module.source.len());

    module.source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

pub(super) fn source_line(module: &ParsedPythonModule, line: usize) -> String {
    module
        .source
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or("")
        .trim()
        .to_owned()
}

pub(super) fn normalize_token(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(super) fn class_name(qualified_name: &str) -> &str {
    qualified_name
        .rsplit_once('.')
        .map_or(qualified_name, |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsing::parse_python_files;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TempProject {
        path: PathBuf,
    }

    impl TempProject {
        fn new(name: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "spelunking-subject-{name}-{}-{stamp}",
                std::process::id()
            ));

            fs::create_dir_all(&path).expect("temp project should be created");

            Self { path }
        }

        fn write(&self, path: &str, contents: &str) -> PathBuf {
            let path = self.path.join(path);

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("parent directories should be created");
            }

            fs::write(&path, contents).expect("temp project file should be written");
            path
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn inspects_django_model_field_structural_radiography() {
        let project = TempProject::new("radiography");
        let models = project.write(
            "reservations/models.py",
            r#"
from django.db import models

class Trip(models.Model):
    name = models.CharField(max_length=100)

class Payment(models.Model):
    pass

class Reservation(models.Model):
    PENDING = "pending"
    CONFIRMED = "confirmed"
    CANCELLED = "cancelled"
    STATUS_CHOICES = (
        (PENDING, "Pending"),
        (CONFIRMED, "Confirmed"),
        (CANCELLED, "Cancelled"),
    )

    trip = models.ForeignKey(Trip, on_delete=models.CASCADE)
    payment = models.OneToOneField("Payment", on_delete=models.CASCADE)
    status = models.CharField(max_length=20, choices=STATUS_CHOICES, default=PENDING)
    notes = models.TextField(blank=True)

    def confirm(self):
        self.status = self.CONFIRMED

    def display_name(self):
        return str(self.trip)
"#,
        );
        let serializers = project.write(
            "reservations/serializers.py",
            r#"
from rest_framework import serializers
from .models import Reservation

class ReservationSerializer(serializers.ModelSerializer):
    class Meta:
        model = Reservation
        fields = ["id", "status"]
"#,
        );
        let views = project.write(
            "reservations/views.py",
            r#"
from rest_framework.viewsets import ModelViewSet
from .models import Reservation
from .serializers import ReservationSerializer

class ReservationViewSet(ModelViewSet):
    queryset = Reservation.objects.all()
    serializer_class = ReservationSerializer
"#,
        );
        let urls = project.write(
            "reservations/urls.py",
            r#"
from django.urls import path
from .views import ReservationViewSet

urlpatterns = [
    path("reservations/", ReservationViewSet.as_view({"get": "list"})),
]
"#,
        );
        let report = parse_python_files(&[models, serializers, views, urls]);
        let subject = inspect_django_subject(&project.path, &report.modules, "Reservation.status")
            .expect("subject inspection should succeed");

        assert_eq!(
            subject.model.as_ref().map(|model| model.name.as_str()),
            Some("Reservation")
        );
        assert!(subject.fields.iter().any(|field| field.name == "status"));
        assert!(
            subject
                .related_models
                .iter()
                .any(|relationship| relationship.model == "Trip")
        );
        assert!(
            subject
                .lifecycle_candidate
                .as_ref()
                .expect("status should be a lifecycle candidate")
                .states
                .iter()
                .any(|state| state.value == "cancelled")
        );
        assert!(
            subject
                .relevant_methods
                .iter()
                .any(|method| method.name == "confirm")
        );
        assert!(
            subject
                .related_components
                .iter()
                .any(|component| component.name == "ReservationSerializer")
        );
        assert!(
            subject
                .related_components
                .iter()
                .any(|component| component.name == "ReservationViewSet")
        );
    }

    #[test]
    fn rejects_ambiguous_short_model_subjects() {
        let project = TempProject::new("ambiguous-subject");
        let web_ticket = project.write(
            "dynamic_pricing/apps/web/models/ticket.py",
            r#"
from django.db import models

class Ticket(models.Model):
    status = models.CharField(max_length=20)
"#,
        );
        let billing_ticket = project.write(
            "dynamic_pricing/apps/billing/models/ticket.py",
            r#"
from django.db import models

class Ticket(models.Model):
    status = models.CharField(max_length=20)
"#,
        );
        let report = parse_python_files(&[web_ticket, billing_ticket]);
        let error = inspect_django_subject(&project.path, &report.modules, "Ticket.status")
            .expect_err("short subject should be ambiguous");

        match error {
            DjangoSubjectError::AmbiguousSubject {
                subject,
                candidates,
            } => {
                assert_eq!(subject, "Ticket.status");
                assert_eq!(candidates.len(), 2);
                assert!(
                    candidates
                        .iter()
                        .any(|candidate| candidate.subject == "web.Ticket.status")
                );
                assert!(
                    candidates
                        .iter()
                        .any(|candidate| candidate.subject == "billing.Ticket.status")
                );
            }
            DjangoSubjectError::InvalidSubject(_) => panic!("expected ambiguous subject error"),
        }
    }

    #[test]
    fn resolves_app_qualified_subjects() {
        let project = TempProject::new("qualified-subject");
        let web_ticket = project.write(
            "dynamic_pricing/apps/web/models/ticket.py",
            r#"
from django.db import models

class Ticket(models.Model):
    status = models.CharField(max_length=20)
"#,
        );
        let billing_ticket = project.write(
            "dynamic_pricing/apps/billing/models/ticket.py",
            r#"
from django.db import models

class Ticket(models.Model):
    status = models.CharField(max_length=20)
"#,
        );
        let report = parse_python_files(&[web_ticket, billing_ticket]);
        let subject =
            inspect_django_subject(&project.path, &report.modules, "billing.Ticket.status")
                .expect("app-qualified subject should resolve");

        assert_eq!(
            subject.model.as_ref().map(|model| model.path.as_str()),
            Some("dynamic_pricing/apps/billing/models/ticket.py")
        );
    }

    #[test]
    fn inspects_django_behavior_mutation_sites_and_paths() {
        let project = TempProject::new("behavior");
        let models = project.write(
            "reservations/models.py",
            r#"
from django.db import models

class Reservation(models.Model):
    PENDING = "pending"
    CANCELLED = "cancelled"
    EXPIRED = "expired"
    STATUS_CHOICES = (
        (PENDING, "Pending"),
        (CANCELLED, "Cancelled"),
        (EXPIRED, "Expired"),
    )

    status = models.CharField(max_length=20, choices=STATUS_CHOICES, default=PENDING)

    def cancel(self):
        self.status = self.CANCELLED

    def persist_status(self):
        self.save(update_fields=["status"])
"#,
        );
        let serializers = project.write(
            "reservations/serializers.py",
            r#"
from rest_framework import serializers
from .models import Reservation

class ReservationSerializer(serializers.ModelSerializer):
    class Meta:
        model = Reservation
        fields = ["id", "status"]
"#,
        );
        let views = project.write(
            "reservations/views.py",
            r#"
from rest_framework.decorators import action
from rest_framework.viewsets import ModelViewSet
from .models import Reservation
from .serializers import ReservationSerializer

class ReservationViewSet(ModelViewSet):
    serializer_class = ReservationSerializer

    @action(detail=True, methods=["post"])
    def cancel(self, request, pk=None):
        reservation = Reservation.objects.get(pk=pk)
        reservation.status = Reservation.CANCELLED
        reservation.save()

def reservation_webhook(request):
    reservation = Reservation.objects.get(pk=request.POST["id"])
    setattr(reservation, "status", Reservation.CANCELLED)
"#,
        );
        let urls = project.write(
            "reservations/urls.py",
            r#"
from django.urls import path
from .views import ReservationViewSet

urlpatterns = [
    path("reservations/<int:pk>/cancel/", ReservationViewSet.as_view({"post": "cancel"})),
]
"#,
        );
        let tasks = project.write(
            "reservations/tasks.py",
            r#"
from celery import shared_task
from .models import Reservation

@shared_task
def expire_pending_reservations():
    Reservation.objects.filter(status=Reservation.PENDING).update(status=Reservation.EXPIRED)
    Reservation.objects.bulk_update([], ["status"])
"#,
        );
        let signals = project.write(
            "reservations/signals.py",
            r#"
from django.db.models.signals import pre_save
from django.dispatch import receiver
from .models import Reservation

@receiver(pre_save, sender=Reservation)
def normalize_reservation_status(sender, instance, **kwargs):
    if isinstance(instance, Reservation):
        instance.status = Reservation.PENDING
"#,
        );
        let admin = project.write(
            "reservations/admin.py",
            r#"
from django.contrib import admin
from .models import Reservation

@admin.action(description="Cancel reservations")
def mark_cancelled(modeladmin, request, queryset):
    queryset.update(status=Reservation.CANCELLED)
"#,
        );
        let report = parse_python_files(&[models, serializers, views, urls, tasks, signals, admin]);
        let behavior =
            inspect_django_behavior(&project.path, &report.modules, "Reservation.status")
                .expect("behavior inspection should succeed");

        assert!(
            behavior
                .mutation_sites
                .iter()
                .any(|site| site.container_kind == "view"
                    && site.kind == "direct_assignment"
                    && site.mutation.contains("reservation.status"))
        );
        assert!(
            behavior
                .mutation_sites
                .iter()
                .any(|site| site.container_kind == "task" && site.kind == "queryset_update")
        );
        assert!(
            behavior
                .mutation_sites
                .iter()
                .any(|site| site.container_kind == "task" && site.kind == "bulk_update")
        );
        assert!(
            behavior
                .mutation_sites
                .iter()
                .any(|site| site.container_kind == "webhook" && site.kind == "setattr")
        );
        assert!(behavior.mutation_sites.iter().any(|site| {
            site.container_kind == "model_method" && site.kind == "save_update_fields"
        }));
        assert!(behavior.mutation_sites.iter().any(|site| {
            site.container_kind == "signal_handler" && site.kind == "direct_assignment"
        }));
        assert!(
            behavior
                .mutation_sites
                .iter()
                .any(|site| site.container_kind == "admin_action" && site.kind == "queryset_update")
        );
        assert!(behavior.behavior_paths.iter().any(|path| {
            path.kind == "api_path"
                && path
                    .steps
                    .iter()
                    .any(|step| step.kind == "route" && step.name.contains("POST"))
        }));
        assert!(
            behavior
                .behavior_paths
                .iter()
                .any(|path| path.kind == "async_path"
                    && path.steps.iter().any(|step| step.kind == "task"))
        );
    }

    #[test]
    fn inspects_django_guidance_risks_questions_tests_and_reading_path() {
        let project = TempProject::new("guidance");
        let models = project.write(
            "reservations/models.py",
            r#"
from django.db import models

class Reservation(models.Model):
    PENDING = "pending"
    CANCELLED = "cancelled"
    CONFIRMED = "confirmed"
    STATUS_CHOICES = (
        (PENDING, "Pending"),
        (CANCELLED, "Cancelled"),
        (CONFIRMED, "Confirmed"),
    )

    status = models.CharField(max_length=20, choices=STATUS_CHOICES, default=PENDING)

    def cancel(self):
        self.status = self.CANCELLED
"#,
        );
        let serializers = project.write(
            "reservations/serializers.py",
            r#"
from rest_framework import serializers
from .models import Reservation

class ReservationSerializer(serializers.ModelSerializer):
    class Meta:
        model = Reservation
        fields = ["id", "status"]
"#,
        );
        let views = project.write(
            "reservations/views.py",
            r#"
from rest_framework.viewsets import ModelViewSet
from .models import Reservation
from .serializers import ReservationSerializer

class ReservationViewSet(ModelViewSet):
    serializer_class = ReservationSerializer

    def cancel(self, request, pk=None):
        reservation = Reservation.objects.get(pk=pk)
        reservation.status = Reservation.CANCELLED
        reservation.save()
"#,
        );
        let urls = project.write(
            "reservations/urls.py",
            r#"
from django.urls import path
from .views import ReservationViewSet

urlpatterns = [
    path("reservations/<int:pk>/cancel/", ReservationViewSet.as_view({"post": "cancel"})),
]
"#,
        );
        let tasks = project.write(
            "reservations/tasks.py",
            r#"
from celery import shared_task
from .models import Reservation

@shared_task
def expire_pending_reservations():
    Reservation.objects.filter(status=Reservation.PENDING).update(status=Reservation.CANCELLED)
"#,
        );
        let admin = project.write(
            "reservations/admin.py",
            r#"
from django.contrib import admin
from .models import Reservation

@admin.action(description="Cancel reservations")
def mark_cancelled(modeladmin, request, queryset):
    queryset.update(status=Reservation.CANCELLED)
"#,
        );
        let webhooks = project.write(
            "payments/webhooks.py",
            r#"
from reservations.models import Reservation

def payment_webhook(request):
    reservation = Reservation.objects.get(pk=request.POST["reservation_id"])
    reservation.status = Reservation.CONFIRMED
    reservation.save()
"#,
        );
        let cancellation_tests = project.write(
            "reservations/tests/test_cancellation.py",
            r#"
from reservations.models import Reservation

def test_cancel_sets_status(db):
    reservation = Reservation.objects.create(status=Reservation.PENDING)
    reservation.cancel()
    assert reservation.status == Reservation.CANCELLED
"#,
        );
        let webhook_tests = project.write(
            "payments/tests/test_webhooks.py",
            r#"
from reservations.models import Reservation

def test_payment_confirms_reservation(db):
    reservation = Reservation.objects.create(status=Reservation.PENDING)
    assert reservation.status != Reservation.CONFIRMED
"#,
        );
        let report = parse_python_files(&[
            models,
            serializers,
            views,
            urls,
            tasks,
            admin,
            webhooks,
            cancellation_tests,
            webhook_tests,
        ]);
        let guidance =
            inspect_django_guidance(&project.path, &report.modules, "Reservation.status")
                .expect("guidance inspection should succeed");

        assert_eq!(
            guidance.analysis_basis.scope,
            "subject-focused behavioral slice"
        );
        assert_eq!(guidance.analysis_basis.subject_slice.related_tests, 2);
        assert!(
            guidance
                .analysis_basis
                .caveats
                .iter()
                .any(|caveat| caveat.contains("not built from a literal GraphExport subgraph"))
        );
        assert!(guidance.risks.iter().any(|risk| {
            risk.title == "Distributed lifecycle ownership" && risk.severity == "high"
        }));
        assert!(
            guidance
                .risks
                .iter()
                .any(|risk| risk.title == "Admin bypass risk")
        );
        assert!(guidance.open_questions.iter().any(|question| {
            question
                .question
                .contains("Which module should own valid transitions")
        }));
        assert!(guidance.related_tests.iter().any(|test| {
            test.path == "reservations/tests/test_cancellation.py" && test.confidence == "high"
        }));
        assert!(
            guidance
                .coupling_signals
                .iter()
                .any(|signal| signal.kind == "cross_app_mutation")
        );
        assert_eq!(
            guidance
                .reading_path
                .first()
                .map(|entry| entry.path.as_str()),
            Some("reservations/models.py")
        );
        assert!(
            guidance
                .reading_path
                .iter()
                .any(|entry| entry.path == "payments/webhooks.py")
        );

        let artifacts =
            build_django_artifact_bundle(&project.path, &report.modules, "Reservation.status")
                .expect("artifact generation should succeed");
        assert_eq!(artifacts.evidence_pack.schema_version, 1);
        assert_eq!(artifacts.evidence_pack.subject, "Reservation.status");
        assert!(artifacts.evidence_pack.mutation_sites.iter().any(|site| {
            site.path == "payments/webhooks.py" && site.mutation.contains("status")
        }));
        assert!(
            artifacts
                .markdown_report
                .contains("# Reservation.status Lifecycle Report")
        );
        assert!(
            artifacts
                .markdown_report
                .contains("## Recommended Reading Path")
        );
        assert!(
            artifacts
                .evaluation_report
                .contains("## Comparison Scorecard")
        );
        assert_eq!(
            django_subject_slug("reservations.Reservation.status"),
            "reservations-reservation-status"
        );
    }
}

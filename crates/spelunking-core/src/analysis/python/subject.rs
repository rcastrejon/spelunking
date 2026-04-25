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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoBehaviorReport {
    pub subject: String,
    pub model: Option<DjangoSubjectModel>,
    pub mutation_sites: Vec<DjangoMutationSite>,
    pub behavior_paths: Vec<DjangoBehaviorPath>,
    pub evidence: Vec<DjangoSubjectEvidence>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoMutationSite {
    pub kind: String,
    pub container_kind: String,
    pub container_name: String,
    pub path: String,
    pub line: usize,
    pub evidence: String,
    pub mutation: String,
    pub value: Option<String>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoBehaviorPath {
    pub kind: String,
    pub steps: Vec<DjangoBehaviorStep>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DjangoBehaviorStep {
    pub kind: String,
    pub name: String,
    pub path: String,
    pub line: usize,
    pub evidence: String,
}

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
struct SubjectParts {
    raw: String,
    model_path: String,
    model_name: String,
    field_name: String,
    app_hint: Option<String>,
}

#[derive(Clone)]
struct DiscoveredClass<'a> {
    module: &'a ParsedPythonModule,
    module_path: String,
    python_module: String,
    import_index: ImportIndex,
    class_def: &'a StmtClassDef,
    qualified_name: String,
    python_qualified_name: String,
}

#[derive(Clone)]
struct ModelCandidate<'a> {
    class: DiscoveredClass<'a>,
    score: usize,
    confidence: &'static str,
}

#[derive(Clone)]
struct FunctionContext<'a> {
    module: &'a ParsedPythonModule,
    module_path: String,
    class: Option<DiscoveredClass<'a>>,
    qualified_name: String,
    decorators: Vec<String>,
    body: &'a ast::Suite,
}

#[derive(Debug, Clone)]
struct DjangoBehaviorCallSite {
    method_name: String,
    container_kind: String,
    container_name: String,
    path: String,
    line: usize,
    evidence: String,
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

pub fn inspect_django_behavior(
    root: impl AsRef<Path>,
    modules: &[ParsedPythonModule],
    subject: &str,
) -> Result<DjangoBehaviorReport, DjangoSubjectError> {
    let root = root.as_ref();
    let subject_report = inspect_django_subject(root, modules, subject)?;
    let parts = SubjectParts::parse(subject)?;
    let classes = discover_classes(root, modules);
    let candidates = find_model_candidates(&classes, &parts);
    let candidate = resolve_model_candidate(&parts, candidates)?;

    let Some(candidate) = candidate else {
        return Ok(DjangoBehaviorReport {
            subject: parts.raw,
            model: None,
            mutation_sites: Vec::new(),
            behavior_paths: Vec::new(),
            evidence: Vec::new(),
            confidence: "low".to_owned(),
        });
    };

    let function_contexts = collect_function_contexts(root, modules, &classes);
    let mut mutation_sites = Vec::new();

    for context in &function_contexts {
        collect_mutation_sites_from_suite(&mut mutation_sites, context, &parts, &candidate.class);
    }

    mutation_sites.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.container_name.cmp(&right.container_name))
            .then(left.mutation.cmp(&right.mutation))
    });
    mutation_sites.dedup_by(|left, right| {
        left.path == right.path
            && left.line == right.line
            && left.container_name == right.container_name
            && left.mutation == right.mutation
    });

    let method_callers = collect_model_method_callers(
        &function_contexts,
        &mutation_sites,
        &parts,
        &candidate.class,
    );
    let behavior_paths = build_behavior_paths(
        root,
        modules,
        &subject_report,
        &mutation_sites,
        &method_callers,
    );
    let evidence = behavior_evidence(&mutation_sites, &behavior_paths);
    let confidence = behavior_confidence(&mutation_sites, &behavior_paths, &candidate.class);

    Ok(DjangoBehaviorReport {
        subject: parts.raw,
        model: subject_report.model,
        mutation_sites,
        behavior_paths,
        evidence,
        confidence,
    })
}

impl SubjectParts {
    fn parse(subject: &str) -> Result<Self, DjangoSubjectError> {
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

fn discover_classes<'a>(
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

fn find_model_candidates<'a>(
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

fn resolve_model_candidate<'a>(
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

fn serializer_or_form_component(
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

fn view_component(
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

fn route_function_name(name: &str) -> bool {
    matches!(class_name(name), "path" | "re_path" | "url")
}

fn route_target_name(expr: &Expr) -> Option<String> {
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

fn class_is_view(class: &DiscoveredClass<'_>) -> bool {
    class.qualified_name.ends_with("View")
        || class.qualified_name.ends_with("ViewSet")
        || class.class_def.bases.iter().any(|base| {
            let name = resolved_base_name(base, &class.import_index);
            name.ends_with("View") || name.ends_with("ViewSet") || name.ends_with("APIView")
        })
}

fn resolved_base_name(expr: &Expr, import_index: &ImportIndex) -> String {
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

fn keyword_arg_expr<'a>(keywords: &'a [ast::Keyword], name: &str) -> Option<&'a Expr> {
    keywords
        .iter()
        .find(|keyword| keyword.arg.as_ref().is_some_and(|arg| arg.as_str() == name))
        .map(|keyword| &keyword.value)
}

fn string_constant(expr: &Expr) -> Option<String> {
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

fn suite_references_model(suite: &ast::Suite, model_name: &str) -> bool {
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

fn expr_references_model(expr: &Expr, model_name: &str) -> bool {
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

fn collect_function_contexts<'a>(
    root: &Path,
    modules: &'a [ParsedPythonModule],
    classes: &[DiscoveredClass<'a>],
) -> Vec<FunctionContext<'a>> {
    let mut contexts = Vec::new();

    for module in modules {
        let module_path = relative_path_identifier(root, &module.path);

        if is_test_module_path(&module_path) {
            continue;
        }

        let python_module = python_module_path(&module_path);
        let import_index = ImportIndex::from_suite(&module.ast, &python_module);

        for statement in &module.ast {
            match statement {
                Stmt::FunctionDef(function_def) => contexts.push(function_context_from_def(
                    module,
                    &module_path,
                    &import_index,
                    None,
                    function_def,
                )),
                Stmt::AsyncFunctionDef(function_def) => {
                    contexts.push(async_function_context_from_def(
                        module,
                        &module_path,
                        &import_index,
                        None,
                        function_def,
                    ))
                }
                _ => {}
            }
        }
    }

    for class in classes {
        for statement in &class.class_def.body {
            match statement {
                Stmt::FunctionDef(function_def) => contexts.push(function_context_from_def(
                    class.module,
                    &class.module_path,
                    &class.import_index,
                    Some(class.clone()),
                    function_def,
                )),
                Stmt::AsyncFunctionDef(function_def) => {
                    contexts.push(async_function_context_from_def(
                        class.module,
                        &class.module_path,
                        &class.import_index,
                        Some(class.clone()),
                        function_def,
                    ))
                }
                _ => {}
            }
        }
    }

    contexts
}

fn function_context_from_def<'a>(
    module: &'a ParsedPythonModule,
    module_path: &str,
    import_index: &ImportIndex,
    class: Option<DiscoveredClass<'a>>,
    function_def: &'a StmtFunctionDef,
) -> FunctionContext<'a> {
    let name = function_def.name.to_string();
    let qualified_name = class
        .as_ref()
        .map(|class| format!("{}.{}", class.qualified_name, name))
        .unwrap_or_else(|| name.clone());

    FunctionContext {
        module,
        module_path: module_path.to_owned(),
        class,
        qualified_name,
        decorators: decorator_names(import_index, &function_def.decorator_list),
        body: &function_def.body,
    }
}

fn async_function_context_from_def<'a>(
    module: &'a ParsedPythonModule,
    module_path: &str,
    import_index: &ImportIndex,
    class: Option<DiscoveredClass<'a>>,
    function_def: &'a StmtAsyncFunctionDef,
) -> FunctionContext<'a> {
    let name = function_def.name.to_string();
    let qualified_name = class
        .as_ref()
        .map(|class| format!("{}.{}", class.qualified_name, name))
        .unwrap_or_else(|| name.clone());

    FunctionContext {
        module,
        module_path: module_path.to_owned(),
        class,
        qualified_name,
        decorators: decorator_names(import_index, &function_def.decorator_list),
        body: &function_def.body,
    }
}

fn decorator_names(import_index: &ImportIndex, decorators: &[Expr]) -> Vec<String> {
    decorators
        .iter()
        .filter_map(|decorator| match decorator {
            Expr::Call(call) => expr_dotted_name(&call.func),
            _ => expr_dotted_name(decorator),
        })
        .map(|name| import_index.resolve(&name))
        .collect()
}

fn collect_mutation_sites_from_suite(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
) {
    for statement in context.body {
        collect_mutation_sites_from_stmt(sites, context, parts, model_class, statement);
    }
}

fn collect_mutation_sites_from_stmt(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    statement: &Stmt,
) {
    match statement {
        Stmt::Assign(assign) => {
            for target in &assign.targets {
                push_assignment_mutation(sites, context, parts, model_class, target, &assign.value);
            }

            collect_mutation_sites_from_expr(sites, context, parts, model_class, &assign.value);
        }
        Stmt::AnnAssign(assign) => {
            if let Some(value) = &assign.value {
                push_assignment_mutation(sites, context, parts, model_class, &assign.target, value);
                collect_mutation_sites_from_expr(sites, context, parts, model_class, value);
            }
        }
        Stmt::AugAssign(assign) => {
            push_assignment_mutation(
                sites,
                context,
                parts,
                model_class,
                &assign.target,
                &assign.value,
            );
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &assign.value);
        }
        Stmt::Expr(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.value);
        }
        Stmt::Return(statement) => {
            if let Some(value) = &statement.value {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, value);
            }
        }
        Stmt::If(statement) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &statement.test);
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.body,
            );
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.orelse,
            );
        }
        Stmt::For(statement) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &statement.iter);
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.body,
            );
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.orelse,
            );
        }
        Stmt::AsyncFor(statement) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &statement.iter);
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.body,
            );
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.orelse,
            );
        }
        Stmt::While(statement) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &statement.test);
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.body,
            );
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.orelse,
            );
        }
        Stmt::With(statement) => {
            for item in &statement.items {
                collect_mutation_sites_from_expr(
                    sites,
                    context,
                    parts,
                    model_class,
                    &item.context_expr,
                );
            }

            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.body,
            );
        }
        Stmt::AsyncWith(statement) => {
            for item in &statement.items {
                collect_mutation_sites_from_expr(
                    sites,
                    context,
                    parts,
                    model_class,
                    &item.context_expr,
                );
            }

            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.body,
            );
        }
        Stmt::Try(statement) => {
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.body,
            );

            for handler in &statement.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                collect_mutation_sites_from_nested_suite(
                    sites,
                    context,
                    parts,
                    model_class,
                    &handler.body,
                );
            }

            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.orelse,
            );
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.finalbody,
            );
        }
        Stmt::TryStar(statement) => {
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.body,
            );

            for handler in &statement.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                collect_mutation_sites_from_nested_suite(
                    sites,
                    context,
                    parts,
                    model_class,
                    &handler.body,
                );
            }

            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.orelse,
            );
            collect_mutation_sites_from_nested_suite(
                sites,
                context,
                parts,
                model_class,
                &statement.finalbody,
            );
        }
        Stmt::Raise(statement) => {
            if let Some(exc) = &statement.exc {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, exc);
            }

            if let Some(cause) = &statement.cause {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, cause);
            }
        }
        Stmt::Assert(statement) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &statement.test);

            if let Some(message) = &statement.msg {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, message);
            }
        }
        Stmt::Delete(statement) => {
            for target in &statement.targets {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, target);
            }
        }
        Stmt::TypeAlias(statement) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &statement.value);
        }
        Stmt::FunctionDef(_)
        | Stmt::AsyncFunctionDef(_)
        | Stmt::ClassDef(_)
        | Stmt::Import(_)
        | Stmt::ImportFrom(_)
        | Stmt::Global(_)
        | Stmt::Nonlocal(_)
        | Stmt::Pass(_)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Match(_) => {}
    }
}

fn collect_mutation_sites_from_nested_suite(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    suite: &ast::Suite,
) {
    for statement in suite {
        collect_mutation_sites_from_stmt(sites, context, parts, model_class, statement);
    }
}

fn push_assignment_mutation(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    target: &Expr,
    value: &Expr,
) {
    let Some(confidence) = assignment_target_confidence(target, context, parts, model_class) else {
        return;
    };

    let line = line_for_node(context.module, target);
    let target_name = expr_dotted_name(target).unwrap_or_else(|| parts.field_name.clone());
    let value = expression_label(context.module, value);

    sites.push(DjangoMutationSite {
        kind: "direct_assignment".to_owned(),
        container_kind: context_kind(context, parts, model_class),
        container_name: context.qualified_name.clone(),
        path: context.module_path.clone(),
        line,
        evidence: source_line(context.module, line),
        mutation: format!("{target_name} = {value}"),
        value: Some(value),
        confidence,
    });
}

fn assignment_target_confidence(
    target: &Expr,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
) -> Option<String> {
    let Expr::Attribute(attribute) = target else {
        return None;
    };

    if attribute.attr.as_str() != parts.field_name {
        return None;
    }

    let owner = expr_dotted_name(&attribute.value)?;

    if owner == "self"
        && context
            .class
            .as_ref()
            .is_some_and(|class| class.python_qualified_name == model_class.python_qualified_name)
    {
        return Some("high".to_owned());
    }

    if owner_suggests_model_instance(&owner, &parts.model_name) {
        return Some("high".to_owned());
    }

    if likely_instance_alias(&owner) && context_mentions_subject(context, parts) {
        return Some("medium".to_owned());
    }

    None
}

fn collect_mutation_sites_from_expr(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    expr: &Expr,
) {
    match expr {
        Expr::Call(call) => {
            push_queryset_update_mutation(sites, context, parts, model_class, call);
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &call.func);

            for arg in &call.args {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, arg);
            }

            for keyword in &call.keywords {
                collect_mutation_sites_from_expr(
                    sites,
                    context,
                    parts,
                    model_class,
                    &keyword.value,
                );
            }
        }
        Expr::NamedExpr(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.target);
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.value);
        }
        Expr::BoolOp(expr) => {
            for value in &expr.values {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, value);
            }
        }
        Expr::BinOp(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.left);
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.right);
        }
        Expr::UnaryOp(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.operand);
        }
        Expr::IfExp(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.test);
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.body);
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.orelse);
        }
        Expr::Dict(expr) => {
            for key in expr.keys.iter().flatten() {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, key);
            }

            for value in &expr.values {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, value);
            }
        }
        Expr::List(expr) => {
            for value in &expr.elts {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, value);
            }
        }
        Expr::Tuple(expr) => {
            for value in &expr.elts {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, value);
            }
        }
        Expr::Set(expr) => {
            for value in &expr.elts {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, value);
            }
        }
        Expr::Compare(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.left);

            for comparator in &expr.comparators {
                collect_mutation_sites_from_expr(sites, context, parts, model_class, comparator);
            }
        }
        Expr::Await(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.value);
        }
        Expr::Attribute(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.value);
        }
        Expr::Subscript(expr) => {
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.value);
            collect_mutation_sites_from_expr(sites, context, parts, model_class, &expr.slice);
        }
        _ => {}
    }
}

fn push_queryset_update_mutation(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    call: &ast::ExprCall,
) {
    let Expr::Attribute(function) = call.func.as_ref() else {
        return;
    };

    if function.attr.as_str() != "update" {
        return;
    }

    let Some(keyword) = call.keywords.iter().find(|keyword| {
        keyword
            .arg
            .as_ref()
            .is_some_and(|arg| arg.as_str() == parts.field_name)
    }) else {
        return;
    };

    let confidence = if expr_references_model(&function.value, &parts.model_name) {
        "high"
    } else if context_mentions_subject(context, parts) {
        "medium"
    } else {
        return;
    };
    let line = line_for_node(context.module, call);
    let receiver = expression_label(context.module, &function.value);
    let value = expression_label(context.module, &keyword.value);

    sites.push(DjangoMutationSite {
        kind: "queryset_update".to_owned(),
        container_kind: context_kind(context, parts, model_class),
        container_name: context.qualified_name.clone(),
        path: context.module_path.clone(),
        line,
        evidence: source_line(context.module, line),
        mutation: format!("{receiver}.update({}={value})", parts.field_name),
        value: Some(value),
        confidence: confidence.to_owned(),
    });
}

fn context_kind(
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
) -> String {
    if context
        .class
        .as_ref()
        .is_some_and(|class| class.python_qualified_name == model_class.python_qualified_name)
    {
        return "model_method".to_owned();
    }

    if is_task_context(context) {
        return "task".to_owned();
    }

    if is_signal_context(context) {
        return "signal_handler".to_owned();
    }

    if is_admin_context(context) {
        return "admin_action".to_owned();
    }

    if is_webhook_context(context) {
        return "webhook".to_owned();
    }

    if is_management_command_context(context) {
        return "management_command".to_owned();
    }

    if let Some(class) = &context.class {
        if serializer_or_form_component(class, parts)
            .is_some_and(|component| component.kind == "serializer")
        {
            return "serializer".to_owned();
        }

        if serializer_or_form_component(class, parts)
            .is_some_and(|component| component.kind == "form")
        {
            return "form".to_owned();
        }

        if view_component(class, parts, &HashSet::new()).is_some() || class_is_view(class) {
            return "view".to_owned();
        }
    }

    "function".to_owned()
}

fn is_task_context(context: &FunctionContext<'_>) -> bool {
    context.module_path.ends_with("tasks.py")
        || context.module_path.contains("/tasks/")
        || context.decorators.iter().any(|decorator| {
            matches!(class_name(decorator), "shared_task" | "task") || decorator.ends_with(".task")
        })
}

fn is_signal_context(context: &FunctionContext<'_>) -> bool {
    context.module_path.ends_with("signals.py")
        || context.module_path.contains("/signals/")
        || context
            .decorators
            .iter()
            .any(|decorator| class_name(decorator) == "receiver")
}

fn is_admin_context(context: &FunctionContext<'_>) -> bool {
    context.module_path.ends_with("admin.py")
        || context.module_path.contains("/admin/")
        || context
            .class
            .as_ref()
            .is_some_and(|class| class_name(&class.qualified_name).ends_with("Admin"))
}

fn is_webhook_context(context: &FunctionContext<'_>) -> bool {
    let path = context.module_path.to_ascii_lowercase();
    let name = context.qualified_name.to_ascii_lowercase();

    path.contains("webhook") || name.contains("webhook")
}

fn is_management_command_context(context: &FunctionContext<'_>) -> bool {
    context.module_path.contains("/management/commands/")
}

fn context_mentions_subject(context: &FunctionContext<'_>, parts: &SubjectParts) -> bool {
    suite_references_model(context.body, &parts.model_name)
        || context
            .class
            .as_ref()
            .is_some_and(|class| serializer_or_form_component(class, parts).is_some())
        || context
            .class
            .as_ref()
            .is_some_and(|class| view_component(class, parts, &HashSet::new()).is_some())
}

fn is_test_module_path(module_path: &str) -> bool {
    module_path.starts_with("tests/")
        || module_path.contains("/tests/")
        || module_path.rsplit('/').next().is_some_and(|file_name| {
            file_name.starts_with("test_") || file_name.ends_with("_test.py")
        })
}

fn owner_suggests_model_instance(owner: &str, model_name: &str) -> bool {
    let owner = owner.rsplit_once('.').map_or(owner, |(_, name)| name);
    let normalized_owner = normalize_token(owner);
    let normalized_model = normalize_token(model_name);

    normalized_owner == normalized_model || normalized_owner.ends_with(&normalized_model)
}

fn likely_instance_alias(owner: &str) -> bool {
    matches!(
        owner.rsplit_once('.').map_or(owner, |(_, name)| name),
        "instance" | "obj" | "object" | "record" | "item"
    )
}

fn expression_label(module: &ParsedPythonModule, expr: &Expr) -> String {
    string_constant(expr)
        .or_else(|| expr_dotted_name(expr))
        .or_else(|| call_expression_label(expr))
        .unwrap_or_else(|| source_line(module, line_for_node(module, expr)))
}

fn call_expression_label(expr: &Expr) -> Option<String> {
    let Expr::Call(call) = expr else {
        return None;
    };

    expr_dotted_name(&call.func).map(|name| format!("{name}(...)"))
}

fn collect_model_method_callers(
    contexts: &[FunctionContext<'_>],
    mutation_sites: &[DjangoMutationSite],
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
) -> Vec<DjangoBehaviorCallSite> {
    let method_names = mutation_sites
        .iter()
        .filter(|site| site.container_kind == "model_method")
        .filter_map(|site| {
            site.container_name
                .rsplit_once('.')
                .map(|(_, method)| method.to_owned())
        })
        .collect::<HashSet<_>>();
    let mut callers = Vec::new();

    if method_names.is_empty() {
        return callers;
    }

    for context in contexts {
        if context
            .class
            .as_ref()
            .is_some_and(|class| class.python_qualified_name == model_class.python_qualified_name)
        {
            continue;
        }

        collect_model_method_callers_from_suite(
            &mut callers,
            context,
            parts,
            model_class,
            &method_names,
            context.body,
        );
    }

    callers.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.method_name.cmp(&right.method_name))
    });
    callers.dedup_by(|left, right| {
        left.method_name == right.method_name
            && left.container_name == right.container_name
            && left.path == right.path
            && left.line == right.line
    });
    callers
}

fn collect_model_method_callers_from_suite(
    callers: &mut Vec<DjangoBehaviorCallSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    method_names: &HashSet<String>,
    suite: &ast::Suite,
) {
    for statement in suite {
        collect_model_method_callers_from_stmt(
            callers,
            context,
            parts,
            model_class,
            method_names,
            statement,
        );
    }
}

fn collect_model_method_callers_from_stmt(
    callers: &mut Vec<DjangoBehaviorCallSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    method_names: &HashSet<String>,
    statement: &Stmt,
) {
    match statement {
        Stmt::Assign(statement) => {
            collect_model_method_callers_from_expr(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.value,
            );
        }
        Stmt::AnnAssign(statement) => {
            if let Some(value) = &statement.value {
                collect_model_method_callers_from_expr(
                    callers,
                    context,
                    parts,
                    model_class,
                    method_names,
                    value,
                );
            }
        }
        Stmt::Expr(statement) => {
            collect_model_method_callers_from_expr(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.value,
            );
        }
        Stmt::Return(statement) => {
            if let Some(value) = &statement.value {
                collect_model_method_callers_from_expr(
                    callers,
                    context,
                    parts,
                    model_class,
                    method_names,
                    value,
                );
            }
        }
        Stmt::If(statement) => {
            collect_model_method_callers_from_expr(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.test,
            );
            collect_model_method_callers_from_suite(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.body,
            );
            collect_model_method_callers_from_suite(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.orelse,
            );
        }
        Stmt::For(statement) => {
            collect_model_method_callers_from_expr(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.iter,
            );
            collect_model_method_callers_from_suite(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.body,
            );
            collect_model_method_callers_from_suite(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.orelse,
            );
        }
        Stmt::While(statement) => {
            collect_model_method_callers_from_suite(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.body,
            );
            collect_model_method_callers_from_suite(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.orelse,
            );
        }
        Stmt::With(statement) => {
            collect_model_method_callers_from_suite(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.body,
            );
        }
        Stmt::Try(statement) => {
            collect_model_method_callers_from_suite(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &statement.body,
            );

            for handler in &statement.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                collect_model_method_callers_from_suite(
                    callers,
                    context,
                    parts,
                    model_class,
                    method_names,
                    &handler.body,
                );
            }
        }
        _ => {}
    }
}

fn collect_model_method_callers_from_expr(
    callers: &mut Vec<DjangoBehaviorCallSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    method_names: &HashSet<String>,
    expr: &Expr,
) {
    match expr {
        Expr::Call(call) => {
            if let Expr::Attribute(function) = call.func.as_ref()
                && method_names.contains(function.attr.as_str())
                && call_receiver_mentions_subject(&function.value, context, parts)
            {
                let line = line_for_node(context.module, call);

                callers.push(DjangoBehaviorCallSite {
                    method_name: function.attr.to_string(),
                    container_kind: context_kind(context, parts, model_class),
                    container_name: context.qualified_name.clone(),
                    path: context.module_path.clone(),
                    line,
                    evidence: source_line(context.module, line),
                });
            }

            collect_model_method_callers_from_expr(
                callers,
                context,
                parts,
                model_class,
                method_names,
                &call.func,
            );

            for arg in &call.args {
                collect_model_method_callers_from_expr(
                    callers,
                    context,
                    parts,
                    model_class,
                    method_names,
                    arg,
                );
            }

            for keyword in &call.keywords {
                collect_model_method_callers_from_expr(
                    callers,
                    context,
                    parts,
                    model_class,
                    method_names,
                    &keyword.value,
                );
            }
        }
        Expr::List(list) => {
            for item in &list.elts {
                collect_model_method_callers_from_expr(
                    callers,
                    context,
                    parts,
                    model_class,
                    method_names,
                    item,
                );
            }
        }
        Expr::Tuple(tuple) => {
            for item in &tuple.elts {
                collect_model_method_callers_from_expr(
                    callers,
                    context,
                    parts,
                    model_class,
                    method_names,
                    item,
                );
            }
        }
        Expr::Attribute(attribute) => collect_model_method_callers_from_expr(
            callers,
            context,
            parts,
            model_class,
            method_names,
            &attribute.value,
        ),
        _ => {}
    }
}

fn call_receiver_mentions_subject(
    receiver: &Expr,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
) -> bool {
    expr_dotted_name(receiver).is_some_and(|owner| {
        owner_suggests_model_instance(&owner, &parts.model_name)
            || (likely_instance_alias(&owner) && context_mentions_subject(context, parts))
    }) || expr_references_model(receiver, &parts.model_name)
}

fn build_behavior_paths(
    root: &Path,
    modules: &[ParsedPythonModule],
    subject_report: &DjangoSubjectReport,
    mutation_sites: &[DjangoMutationSite],
    method_callers: &[DjangoBehaviorCallSite],
) -> Vec<DjangoBehaviorPath> {
    let mut paths = Vec::new();

    for site in mutation_sites {
        if site.container_kind == "model_method"
            && let Some((_, method_name)) = site.container_name.rsplit_once('.')
        {
            for caller in method_callers
                .iter()
                .filter(|caller| caller.method_name == method_name)
            {
                let mut steps = route_steps_for_container(
                    root,
                    modules,
                    subject_report,
                    &caller.container_kind,
                    &caller.container_name,
                );
                steps.push(DjangoBehaviorStep {
                    kind: caller.container_kind.clone(),
                    name: caller.container_name.clone(),
                    path: caller.path.clone(),
                    line: caller.line,
                    evidence: caller.evidence.clone(),
                });
                steps.push(DjangoBehaviorStep {
                    kind: site.container_kind.clone(),
                    name: site.container_name.clone(),
                    path: site.path.clone(),
                    line: site.line,
                    evidence: site.evidence.clone(),
                });

                if let Some(subject_step) = subject_step(subject_report) {
                    steps.push(subject_step);
                }

                paths.push(DjangoBehaviorPath {
                    kind: behavior_path_kind(&caller.container_kind).to_owned(),
                    confidence: if steps.len() >= 3 { "high" } else { "medium" }.to_owned(),
                    steps,
                });
            }
        }

        let mut steps = route_steps_for_site(root, modules, subject_report, site);
        steps.push(DjangoBehaviorStep {
            kind: site.container_kind.clone(),
            name: site.container_name.clone(),
            path: site.path.clone(),
            line: site.line,
            evidence: site.evidence.clone(),
        });

        if let Some(subject_step) = subject_step(subject_report) {
            steps.push(subject_step);
        }

        paths.push(DjangoBehaviorPath {
            kind: behavior_path_kind(&site.container_kind).to_owned(),
            confidence: if steps.len() >= 3 { "high" } else { "medium" }.to_owned(),
            steps,
        });
    }

    paths.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(
                left.steps
                    .first()
                    .map(|step| &step.path)
                    .cmp(&right.steps.first().map(|step| &step.path)),
            )
            .then(
                left.steps
                    .last()
                    .map(|step| &step.line)
                    .cmp(&right.steps.last().map(|step| &step.line)),
            )
    });
    paths.dedup_by(|left, right| left.kind == right.kind && left.steps == right.steps);
    paths
}

fn route_steps_for_site(
    root: &Path,
    modules: &[ParsedPythonModule],
    subject_report: &DjangoSubjectReport,
    site: &DjangoMutationSite,
) -> Vec<DjangoBehaviorStep> {
    route_steps_for_container(
        root,
        modules,
        subject_report,
        &site.container_kind,
        &site.container_name,
    )
}

fn route_steps_for_container(
    root: &Path,
    modules: &[ParsedPythonModule],
    subject_report: &DjangoSubjectReport,
    container_kind: &str,
    container_name: &str,
) -> Vec<DjangoBehaviorStep> {
    let view_name = container_name.split('.').next().unwrap_or(container_name);
    let mut steps = Vec::new();

    for component in subject_report
        .related_components
        .iter()
        .filter(|component| component.kind == "url")
    {
        if container_kind == "view"
            && !(component.reason.contains(view_name) || component.evidence.contains(view_name))
        {
            continue;
        }

        if !matches!(container_kind, "view" | "serializer" | "form") {
            continue;
        }

        steps.push(DjangoBehaviorStep {
            kind: "route".to_owned(),
            name: component.name.clone(),
            path: component.path.clone(),
            line: component.line,
            evidence: component.evidence.clone(),
        });
        break;
    }

    if steps.is_empty() && matches!(container_kind, "view" | "serializer" | "form") {
        steps.extend(route_steps_from_modules(root, modules, view_name));
    }

    if matches!(container_kind, "serializer" | "form")
        && let Some(component) = subject_report
            .related_components
            .iter()
            .find(|component| component.kind == "view")
    {
        steps.push(DjangoBehaviorStep {
            kind: "view".to_owned(),
            name: component.name.clone(),
            path: component.path.clone(),
            line: component.line,
            evidence: component.evidence.clone(),
        });
    }

    steps
}

fn route_steps_from_modules(
    root: &Path,
    modules: &[ParsedPythonModule],
    view_name: &str,
) -> Vec<DjangoBehaviorStep> {
    let mut steps = Vec::new();

    for module in modules {
        let module_path = relative_path_identifier(root, &module.path);

        if !module_path.ends_with("urls.py") {
            continue;
        }

        for statement in &module.ast {
            collect_route_steps_from_stmt(&mut steps, module, &module_path, view_name, statement);
        }
    }

    steps.sort_by(|left, right| left.path.cmp(&right.path).then(left.line.cmp(&right.line)));
    steps.dedup_by(|left, right| left.path == right.path && left.line == right.line);
    steps.into_iter().take(1).collect()
}

fn collect_route_steps_from_stmt(
    steps: &mut Vec<DjangoBehaviorStep>,
    module: &ParsedPythonModule,
    module_path: &str,
    view_name: &str,
    statement: &Stmt,
) {
    match statement {
        Stmt::Assign(statement) => {
            collect_route_steps_from_expr(steps, module, module_path, view_name, &statement.value);
        }
        Stmt::Expr(statement) => {
            collect_route_steps_from_expr(steps, module, module_path, view_name, &statement.value);
        }
        _ => {}
    }
}

fn collect_route_steps_from_expr(
    steps: &mut Vec<DjangoBehaviorStep>,
    module: &ParsedPythonModule,
    module_path: &str,
    view_name: &str,
    expr: &Expr,
) {
    match expr {
        Expr::Call(call) => {
            if expr_dotted_name(&call.func).is_some_and(|name| route_function_name(&name)) {
                let target = call.args.get(1).and_then(route_target_name);

                if target
                    .as_deref()
                    .is_some_and(|target| class_name(target) == view_name)
                {
                    let route = call
                        .args
                        .first()
                        .and_then(string_constant)
                        .unwrap_or_else(|| "<dynamic route>".to_owned());
                    let line = line_for_node(module, expr);

                    steps.push(DjangoBehaviorStep {
                        kind: "route".to_owned(),
                        name: route,
                        path: module_path.to_owned(),
                        line,
                        evidence: source_line(module, line),
                    });
                }
            }

            for arg in &call.args {
                collect_route_steps_from_expr(steps, module, module_path, view_name, arg);
            }
        }
        Expr::List(list) => {
            for item in &list.elts {
                collect_route_steps_from_expr(steps, module, module_path, view_name, item);
            }
        }
        Expr::Tuple(tuple) => {
            for item in &tuple.elts {
                collect_route_steps_from_expr(steps, module, module_path, view_name, item);
            }
        }
        _ => {}
    }
}

fn subject_step(subject_report: &DjangoSubjectReport) -> Option<DjangoBehaviorStep> {
    let model = subject_report.model.as_ref()?;
    let (line, evidence) = subject_report
        .lifecycle_candidate
        .as_ref()
        .map(|candidate| (candidate.line, candidate.evidence.clone()))
        .unwrap_or_else(|| (model.line, model.evidence.clone()));

    Some(DjangoBehaviorStep {
        kind: "subject".to_owned(),
        name: subject_report.subject.clone(),
        path: model.path.clone(),
        line,
        evidence,
    })
}

fn behavior_path_kind(container_kind: &str) -> &'static str {
    match container_kind {
        "view" | "serializer" | "form" => "api_path",
        "task" => "async_path",
        "signal_handler" => "signal_path",
        "admin_action" => "admin_path",
        "webhook" => "webhook_path",
        "management_command" => "management_path",
        "model_method" => "model_path",
        _ => "behavior_path",
    }
}

fn behavior_evidence(
    mutation_sites: &[DjangoMutationSite],
    behavior_paths: &[DjangoBehaviorPath],
) -> Vec<DjangoSubjectEvidence> {
    let mut evidence = BTreeSet::new();

    for site in mutation_sites {
        evidence.insert(DjangoSubjectEvidence {
            path: site.path.clone(),
            line: site.line,
            detail: format!("{} mutation in {}", site.kind, site.container_name),
        });
    }

    for path in behavior_paths {
        for step in &path.steps {
            evidence.insert(DjangoSubjectEvidence {
                path: step.path.clone(),
                line: step.line,
                detail: format!("{} step {}", path.kind, step.name),
            });
        }
    }

    evidence.into_iter().collect()
}

fn behavior_confidence(
    mutation_sites: &[DjangoMutationSite],
    behavior_paths: &[DjangoBehaviorPath],
    _model_class: &DiscoveredClass<'_>,
) -> String {
    if mutation_sites.is_empty() {
        return "low".to_owned();
    }

    if mutation_sites
        .iter()
        .any(|site| site.container_kind != "model_method")
        && behavior_paths.len() >= 2
    {
        "high".to_owned()
    } else if behavior_paths
        .iter()
        .any(|path| !matches!(path.kind.as_str(), "model_path" | "behavior_path"))
    {
        "high".to_owned()
    } else {
        "medium".to_owned()
    }
}

fn line_for_node<T: Ranged>(module: &ParsedPythonModule, node: &T) -> usize {
    let offset = node.start().to_usize().min(module.source.len());

    module.source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn source_line(module: &ParsedPythonModule, line: usize) -> String {
    module
        .source
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn normalize_token(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn class_name(qualified_name: &str) -> &str {
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
"#,
        );
        let report = parse_python_files(&[models, serializers, views, urls, tasks]);
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
                .behavior_paths
                .iter()
                .any(|path| path.kind == "api_path"
                    && path.steps.iter().any(|step| step.kind == "route"))
        );
        assert!(
            behavior
                .behavior_paths
                .iter()
                .any(|path| path.kind == "async_path"
                    && path.steps.iter().any(|step| step.kind == "task"))
        );
    }
}

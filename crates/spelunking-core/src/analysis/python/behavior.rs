use super::imports::{ImportIndex, expr_dotted_name, python_module_path};
use super::subject::{
    DiscoveredClass, DjangoSubjectError, DjangoSubjectEvidence, DjangoSubjectModel,
    DjangoSubjectReport, SubjectParts, class_is_view, class_name, discover_classes,
    expr_references_model, find_model_candidates, inspect_django_subject, keyword_arg_expr,
    line_for_node, normalize_token, resolve_model_candidate, resolved_base_name,
    route_function_name, route_target_name, serializer_or_form_component, source_line,
    string_constant, suite_references_model, view_component,
};
use crate::{graph::relative_path_identifier, parsing::ParsedPythonModule};
use rustpython_parser::ast::{self, Expr, Stmt, StmtAsyncFunctionDef, StmtFunctionDef};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashSet},
    path::Path,
};

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

#[derive(Clone)]
struct FunctionContext<'a> {
    module: &'a ParsedPythonModule,
    module_path: String,
    class: Option<DiscoveredClass<'a>>,
    qualified_name: String,
    decorators: Vec<String>,
    arg_names: Vec<String>,
    body: &'a ast::Suite,
    is_async: bool,
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
        arg_names: argument_names(&function_def.args),
        body: &function_def.body,
        is_async: false,
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
        arg_names: argument_names(&function_def.args),
        body: &function_def.body,
        is_async: true,
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

fn argument_names(arguments: &ast::Arguments) -> Vec<String> {
    arguments
        .posonlyargs
        .iter()
        .chain(arguments.args.iter())
        .chain(arguments.kwonlyargs.iter())
        .map(|argument| argument.def.arg.to_string())
        .chain(
            arguments
                .vararg
                .iter()
                .map(|argument| argument.arg.to_string()),
        )
        .chain(
            arguments
                .kwarg
                .iter()
                .map(|argument| argument.arg.to_string()),
        )
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
            push_setattr_mutation(sites, context, parts, model_class, call);
            push_bulk_update_mutation(sites, context, parts, model_class, call);
            push_save_update_fields_mutation(sites, context, parts, model_class, call);
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

fn push_setattr_mutation(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    call: &ast::ExprCall,
) {
    if expr_dotted_name(&call.func).as_deref().map(class_name) != Some("setattr") {
        return;
    }

    let Some(target) = call.args.first() else {
        return;
    };
    let Some(field_name) = call.args.get(1).and_then(string_constant) else {
        return;
    };

    if field_name != parts.field_name {
        return;
    }

    let Some(confidence) = receiver_confidence(target, context, parts, model_class) else {
        return;
    };
    let line = line_for_node(context.module, call);
    let value = call
        .args
        .get(2)
        .map(|value| expression_label(context.module, value));

    sites.push(DjangoMutationSite {
        kind: "setattr".to_owned(),
        container_kind: context_kind(context, parts, model_class),
        container_name: context.qualified_name.clone(),
        path: context.module_path.clone(),
        line,
        evidence: source_line(context.module, line),
        mutation: format!(
            "setattr({}, {:?}, {})",
            expression_label(context.module, target),
            field_name,
            value.as_deref().unwrap_or("<unknown>")
        ),
        value,
        confidence,
    });
}

fn push_bulk_update_mutation(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    call: &ast::ExprCall,
) {
    if expr_dotted_name(&call.func).as_deref().map(class_name) != Some("bulk_update") {
        return;
    }

    let fields_expr = call
        .args
        .get(1)
        .or_else(|| keyword_arg_expr(&call.keywords, "fields"));
    let Some(fields_expr) = fields_expr else {
        return;
    };

    if !expr_contains_string(fields_expr, &parts.field_name) {
        return;
    }

    let receiver = bulk_update_receiver(call);
    let confidence = if receiver
        .as_ref()
        .is_some_and(|receiver| expr_references_model(receiver, &parts.model_name))
    {
        "high".to_owned()
    } else if context_mentions_subject(context, parts)
        || call
            .args
            .first()
            .is_some_and(|arg| collection_name_suggests_model(arg, &parts.model_name))
    {
        "medium".to_owned()
    } else {
        return;
    };
    let line = line_for_node(context.module, call);

    sites.push(DjangoMutationSite {
        kind: "bulk_update".to_owned(),
        container_kind: context_kind(context, parts, model_class),
        container_name: context.qualified_name.clone(),
        path: context.module_path.clone(),
        line,
        evidence: source_line(context.module, line),
        mutation: format!("bulk_update(..., fields=[{}])", parts.field_name),
        value: None,
        confidence,
    });
}

fn push_save_update_fields_mutation(
    sites: &mut Vec<DjangoMutationSite>,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
    call: &ast::ExprCall,
) {
    let Expr::Attribute(function) = call.func.as_ref() else {
        return;
    };

    if function.attr.as_str() != "save" {
        return;
    }

    let Some(update_fields) = keyword_arg_expr(&call.keywords, "update_fields") else {
        return;
    };

    if !expr_contains_string(update_fields, &parts.field_name) {
        return;
    }

    let Some(confidence) = receiver_confidence(&function.value, context, parts, model_class) else {
        return;
    };
    let line = line_for_node(context.module, call);
    let receiver = expression_label(context.module, &function.value);

    sites.push(DjangoMutationSite {
        kind: "save_update_fields".to_owned(),
        container_kind: context_kind(context, parts, model_class),
        container_name: context.qualified_name.clone(),
        path: context.module_path.clone(),
        line,
        evidence: source_line(context.module, line),
        mutation: format!("{receiver}.save(update_fields=[{}])", parts.field_name),
        value: None,
        confidence,
    });
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

fn receiver_confidence(
    receiver: &Expr,
    context: &FunctionContext<'_>,
    parts: &SubjectParts,
    model_class: &DiscoveredClass<'_>,
) -> Option<String> {
    if expr_references_model(receiver, &parts.model_name) {
        return Some("high".to_owned());
    }

    let owner = expr_dotted_name(receiver)?;

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

fn bulk_update_receiver(call: &ast::ExprCall) -> Option<&Expr> {
    let Expr::Attribute(function) = call.func.as_ref() else {
        return None;
    };

    Some(&function.value)
}

fn expr_contains_string(expr: &Expr, expected: &str) -> bool {
    match expr {
        Expr::Constant(_) => string_constant(expr).as_deref() == Some(expected),
        Expr::List(list) => list
            .elts
            .iter()
            .any(|value| expr_contains_string(value, expected)),
        Expr::Tuple(tuple) => tuple
            .elts
            .iter()
            .any(|value| expr_contains_string(value, expected)),
        Expr::Set(set) => set
            .elts
            .iter()
            .any(|value| expr_contains_string(value, expected)),
        _ => false,
    }
}

fn collection_name_suggests_model(expr: &Expr, model_name: &str) -> bool {
    expr_dotted_name(expr).is_some_and(|name| owner_suggests_model_instance(&name, model_name))
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

    if context.is_async {
        return "async_function".to_owned();
    }

    "function".to_owned()
}

fn is_task_context(context: &FunctionContext<'_>) -> bool {
    has_task_decorator(context)
        || context.module_path.ends_with("tasks.py")
        || context.module_path.contains("/tasks/")
}

fn is_signal_context(context: &FunctionContext<'_>) -> bool {
    has_receiver_decorator(context)
        || context.module_path.ends_with("signals.py")
        || context.module_path.contains("/signals/")
}

fn is_admin_context(context: &FunctionContext<'_>) -> bool {
    has_admin_action_decorator(context)
        || context.module_path.ends_with("admin.py")
        || context.module_path.contains("/admin/")
        || context
            .class
            .as_ref()
            .is_some_and(|class| class_is_model_admin(class))
}

fn is_webhook_context(context: &FunctionContext<'_>) -> bool {
    let path = context.module_path.to_ascii_lowercase();
    let name = context.qualified_name.to_ascii_lowercase();

    (path.contains("webhook") || name.contains("webhook"))
        && (context.arg_names.iter().any(|arg| arg == "request")
            || context.class.as_ref().is_some_and(class_is_view))
}

fn has_task_decorator(context: &FunctionContext<'_>) -> bool {
    context.decorators.iter().any(|decorator| {
        matches!(class_name(decorator), "shared_task" | "task") || decorator.ends_with(".task")
    })
}

fn has_receiver_decorator(context: &FunctionContext<'_>) -> bool {
    context
        .decorators
        .iter()
        .any(|decorator| class_name(decorator) == "receiver")
}

fn has_admin_action_decorator(context: &FunctionContext<'_>) -> bool {
    context.decorators.iter().any(|decorator| {
        decorator == "django.contrib.admin.action" || decorator.ends_with(".admin.action")
    })
}

fn class_is_model_admin(class: &DiscoveredClass<'_>) -> bool {
    class_name(&class.qualified_name).ends_with("Admin")
        || class.class_def.bases.iter().any(|base| {
            let name = resolved_base_name(base, &class.import_index);
            name.ends_with("ModelAdmin") || name.ends_with(".admin.ModelAdmin")
        })
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

    normalized_owner == normalized_model
        || normalized_owner == format!("{normalized_model}s")
        || normalized_owner.ends_with(&normalized_model)
        || normalized_owner.ends_with(&format!("{normalized_model}s"))
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
                    let route_name = route_display_name(call, &route);
                    let line = line_for_node(module, expr);

                    steps.push(DjangoBehaviorStep {
                        kind: "route".to_owned(),
                        name: route_name,
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

fn route_display_name(call: &ast::ExprCall, route: &str) -> String {
    let Some(target) = call.args.get(1) else {
        return route.to_owned();
    };
    let actions = route_http_actions(target);

    if actions.is_empty() {
        route.to_owned()
    } else {
        actions
            .into_iter()
            .map(|(method, action)| {
                format!("{} {} -> {}", method.to_ascii_uppercase(), route, action)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn route_http_actions(expr: &Expr) -> Vec<(String, String)> {
    let Expr::Call(call) = expr else {
        return Vec::new();
    };

    if expr_dotted_name(&call.func)
        .as_deref()
        .is_none_or(|name| class_name(name) != "as_view")
    {
        return Vec::new();
    }

    let Some(Expr::Dict(actions)) = call.args.first() else {
        return Vec::new();
    };

    actions
        .keys
        .iter()
        .zip(actions.values.iter())
        .filter_map(|(method, action)| {
            Some((
                method.as_ref().and_then(string_constant)?,
                string_constant(action)?,
            ))
        })
        .collect()
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
        "async_function" => "async_path",
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

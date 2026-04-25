use super::imports::{ImportIndex, expr_dotted_name, first_segment, python_module_path};
use crate::{
    analysis::{AnalysisContext, Analyzer, SourceLanguage},
    graph::{EdgeType, GraphBuilder, NodeKey, NodeType, relative_path_identifier},
};
use petgraph::graph::NodeIndex;
use rustpython_parser::ast::{
    self, Constant, Expr, Stmt, StmtAsyncFunctionDef, StmtClassDef, StmtFunctionDef,
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub struct DjangoModelAnalyzer;

impl Analyzer for DjangoModelAnalyzer {
    fn name(&self) -> &'static str {
        "python.django.data_model"
    }

    fn language(&self) -> SourceLanguage {
        SourceLanguage::Python
    }

    fn analyze(&self, context: &AnalysisContext<'_>, graph: &mut GraphBuilder) {
        let mut index = DjangoProjectIndex::from_context(context);
        index.resolve_model_classes();
        index.resolve_routes();
        index.emit_graph(graph);
    }
}

#[derive(Debug, Clone)]
struct ClassSymbol {
    source_file: Option<NodeIndex>,
    module_path: String,
    python_module: String,
    qualified_name: String,
    python_qualified_name: String,
    bases: Vec<ModelReference>,
    relationships: Vec<ModelRelationship>,
    app: Option<DjangoApp>,
}

impl ClassSymbol {
    fn model_identifier(&self) -> String {
        format!("{}:{}", self.module_path, self.qualified_name)
    }

    fn model_node_key(&self) -> NodeKey {
        NodeKey::new(NodeType::Model, self.model_identifier())
    }
}

#[derive(Debug, Clone)]
struct ViewSymbol {
    source_file: Option<NodeIndex>,
    module_path: String,
    qualified_name: String,
    python_qualified_name: String,
}

impl ViewSymbol {
    fn node_key(&self) -> NodeKey {
        NodeKey::new(NodeType::View, self.python_qualified_name.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DjangoApp {
    identifier: String,
    label: String,
    path: String,
}

impl DjangoApp {
    fn node_key(&self) -> NodeKey {
        NodeKey::new(NodeType::App, self.identifier.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelReference {
    value: String,
}

impl ModelReference {
    fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    fn is_direct_django_model(&self) -> bool {
        self.value == "django.db.models.Model"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelRelationship {
    target: ModelReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ViewReference {
    value: String,
}

impl ViewReference {
    fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct RawRoutePattern {
    source_file: Option<NodeIndex>,
    source_path: String,
    python_module: String,
    route: String,
    target: RawRouteTarget,
    ordinal: usize,
}

#[derive(Debug, Clone)]
enum RawRouteTarget {
    View(ViewReference),
    Include(IncludeReference),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum IncludeReference {
    Module(String),
    Router {
        python_module: String,
        variable: String,
    },
}

#[derive(Debug, Clone)]
struct ResolvedRoutePattern {
    source_file: Option<NodeIndex>,
    source_path: String,
    route: String,
    view: ViewReference,
    ordinal: usize,
}

#[derive(Debug, Clone)]
struct RouterRegistration {
    route: String,
    view: ViewReference,
}

#[derive(Debug, Default)]
struct DjangoProjectIndex {
    classes: Vec<ClassSymbol>,
    classes_by_python_name: HashMap<String, usize>,
    classes_by_module_local_name: HashMap<(String, String), usize>,
    classes_by_app_and_name: HashMap<(String, String), usize>,
    model_class_indices: HashSet<usize>,
    views: Vec<ViewSymbol>,
    views_by_python_name: HashMap<String, usize>,
    raw_routes: Vec<RawRoutePattern>,
    raw_routes_by_python_module: HashMap<String, Vec<usize>>,
    router_registrations_by_owner: HashMap<(String, String), Vec<RouterRegistration>>,
    routes: Vec<ResolvedRoutePattern>,
}

impl DjangoProjectIndex {
    fn from_context(context: &AnalysisContext<'_>) -> Self {
        let mut index = Self::default();

        for module in context.python_modules() {
            let module_path = relative_path_identifier(context.root(), &module.path);
            let python_module = python_module_path(&module_path);
            let import_index = ImportIndex::from_suite(&module.ast, &python_module);
            let source_file = context.source_file_index(&module.path);

            collect_definitions_in_suite(
                &mut index,
                source_file,
                &module_path,
                &python_module,
                &import_index,
                &module.ast,
                &mut Vec::new(),
            );
            collect_routes_in_suite(
                &mut index,
                source_file,
                &module_path,
                &python_module,
                &import_index,
                &module.ast,
            );
        }

        index
    }

    fn add_class(&mut self, class: ClassSymbol) {
        let class_index = self.classes.len();

        self.classes_by_python_name
            .insert(class.python_qualified_name.clone(), class_index);
        self.classes_by_module_local_name.insert(
            (class.python_module.clone(), class.qualified_name.clone()),
            class_index,
        );
        self.classes_by_module_local_name.insert(
            (
                class.python_module.clone(),
                class_name(&class.qualified_name).to_owned(),
            ),
            class_index,
        );

        if let Some(app) = &class.app {
            self.classes_by_app_and_name.insert(
                (
                    app.label.clone(),
                    class_name(&class.qualified_name).to_owned(),
                ),
                class_index,
            );
        }

        self.classes.push(class);
    }

    fn add_view(&mut self, view: ViewSymbol) {
        let view_index = self.views.len();

        self.views_by_python_name
            .insert(view.python_qualified_name.clone(), view_index);
        self.views.push(view);
    }

    fn add_raw_route(
        &mut self,
        source_file: Option<NodeIndex>,
        source_path: &str,
        python_module: &str,
        route: String,
        target: RawRouteTarget,
    ) {
        let route_index = self.raw_routes.len();

        self.raw_routes.push(RawRoutePattern {
            source_file,
            source_path: source_path.to_owned(),
            python_module: python_module.to_owned(),
            route,
            target,
            ordinal: route_index,
        });
        self.raw_routes_by_python_module
            .entry(python_module.to_owned())
            .or_default()
            .push(route_index);
    }

    fn add_router_registration(
        &mut self,
        python_module: &str,
        variable: &str,
        registration: RouterRegistration,
    ) {
        self.router_registrations_by_owner
            .entry((python_module.to_owned(), variable.to_owned()))
            .or_default()
            .push(registration);
    }

    fn resolve_model_classes(&mut self) {
        let mut resolution_state = vec![ResolutionState::Unvisited; self.classes.len()];

        for class_index in 0..self.classes.len() {
            if self.is_model_class(class_index, &mut resolution_state) {
                self.model_class_indices.insert(class_index);
            }
        }
    }

    fn is_model_class(&self, class_index: usize, state: &mut [ResolutionState]) -> bool {
        match state[class_index] {
            ResolutionState::Resolved(is_model) => return is_model,
            ResolutionState::Visiting => return false,
            ResolutionState::Unvisited => {}
        }

        state[class_index] = ResolutionState::Visiting;

        let is_model = self.classes[class_index].bases.iter().any(|base| {
            base.is_direct_django_model()
                || self
                    .resolve_model_reference(class_index, base)
                    .is_some_and(|base_index| self.is_model_class(base_index, state))
        });

        state[class_index] = ResolutionState::Resolved(is_model);
        is_model
    }

    fn resolve_routes(&mut self) {
        self.routes.clear();

        let included_modules = self
            .raw_routes
            .iter()
            .filter_map(|route| match &route.target {
                RawRouteTarget::Include(IncludeReference::Module(module)) => Some(module.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();

        let root_route_indices = self
            .raw_routes
            .iter()
            .enumerate()
            .filter_map(|(index, route)| {
                (!included_modules.contains(&route.python_module)).then_some(index)
            })
            .collect::<Vec<_>>();

        let mut routes = Vec::new();

        for route_index in root_route_indices {
            self.expand_route(route_index, "", &mut Vec::new(), &mut routes);
        }

        self.routes = routes;
    }

    fn expand_route(
        &self,
        route_index: usize,
        prefix: &str,
        module_stack: &mut Vec<String>,
        routes: &mut Vec<ResolvedRoutePattern>,
    ) {
        let raw_route = &self.raw_routes[route_index];
        let route = combine_route_patterns(prefix, &raw_route.route);

        match &raw_route.target {
            RawRouteTarget::View(view) => routes.push(ResolvedRoutePattern {
                source_file: raw_route.source_file,
                source_path: raw_route.source_path.clone(),
                route,
                view: view.clone(),
                ordinal: raw_route.ordinal,
            }),
            RawRouteTarget::Include(IncludeReference::Module(module)) => {
                if module_stack.contains(module) {
                    return;
                }

                let Some(included_routes) = self.raw_routes_by_python_module.get(module) else {
                    return;
                };

                module_stack.push(module.clone());

                for &included_route_index in included_routes {
                    self.expand_route(included_route_index, &route, module_stack, routes);
                }

                module_stack.pop();
            }
            RawRouteTarget::Include(IncludeReference::Router {
                python_module,
                variable,
            }) => {
                let Some(registrations) = self
                    .router_registrations_by_owner
                    .get(&(python_module.clone(), variable.clone()))
                else {
                    return;
                };

                for registration in registrations {
                    routes.push(ResolvedRoutePattern {
                        source_file: raw_route.source_file,
                        source_path: raw_route.source_path.clone(),
                        route: combine_route_patterns(&route, &registration.route),
                        view: registration.view.clone(),
                        ordinal: raw_route.ordinal,
                    });
                }
            }
        }
    }

    fn emit_graph(&self, graph: &mut GraphBuilder) {
        let mut model_nodes = HashMap::new();
        let mut app_nodes = HashMap::new();
        let mut model_class_indices = self.model_class_indices.iter().copied().collect::<Vec<_>>();

        model_class_indices.sort_unstable();

        for &class_index in &model_class_indices {
            let class = &self.classes[class_index];
            let model = graph.add_node(
                class.model_node_key(),
                class.qualified_name.clone(),
                Some(class.module_path.clone()),
            );

            model_nodes.insert(class_index, model);

            if let Some(source_file) = class.source_file {
                graph.add_edge(source_file, model, EdgeType::Contains);
            }

            if let Some(app) = &class.app {
                let app_node = *app_nodes.entry(app.identifier.clone()).or_insert_with(|| {
                    graph.add_node(app.node_key(), app.label.clone(), Some(app.path.clone()))
                });

                graph.add_edge(app_node, model, EdgeType::Contains);
            }
        }

        for class_index in model_class_indices {
            let model = model_nodes[&class_index];
            let class = &self.classes[class_index];

            for base in &class.bases {
                if let Some(base_index) = self.resolve_model_reference(class_index, base)
                    && let Some(&base_model) = model_nodes.get(&base_index)
                {
                    graph.add_edge(model, base_model, EdgeType::Inherits);
                }
            }

            for relationship in &class.relationships {
                if let Some(target_index) =
                    self.resolve_model_reference(class_index, &relationship.target)
                    && let Some(&target_model) = model_nodes.get(&target_index)
                {
                    graph.add_edge(model, target_model, EdgeType::RelatesTo);
                }
            }
        }

        self.emit_route_graph(graph);
    }

    fn resolve_model_reference(
        &self,
        current_class_index: usize,
        reference: &ModelReference,
    ) -> Option<usize> {
        let current_class = &self.classes[current_class_index];

        if reference.value == "self" {
            return Some(current_class_index);
        }

        if let Some(class_index) = self.classes_by_python_name.get(&reference.value) {
            return Some(*class_index);
        }

        if let Some(class_index) = self
            .classes_by_module_local_name
            .get(&(current_class.python_module.clone(), reference.value.clone()))
        {
            return Some(*class_index);
        }

        if let Some(app_label) = current_class.app.as_ref().map(|app| app.label.as_str())
            && let Some(class_index) = self
                .classes_by_app_and_name
                .get(&(app_label.to_owned(), reference.value.clone()))
        {
            return Some(*class_index);
        }

        if let Some((app_label, model_name)) = reference.value.split_once('.')
            && let Some(class_index) = self
                .classes_by_app_and_name
                .get(&(app_label.to_owned(), model_name.to_owned()))
        {
            return Some(*class_index);
        }

        None
    }

    fn emit_route_graph(&self, graph: &mut GraphBuilder) {
        let mut view_nodes = HashMap::new();

        for route in &self.routes {
            let url = graph.add_node(
                NodeKey::new(
                    NodeType::Url,
                    format!("{}:{}:{}", route.source_path, route.ordinal, route.route),
                ),
                route.route.clone(),
                Some(route.source_path.clone()),
            );

            if let Some(source_file) = route.source_file {
                graph.add_edge(source_file, url, EdgeType::Contains);
            }

            let view = self.view_node_for_reference(graph, &route.view, &mut view_nodes);

            graph.add_edge(url, view, EdgeType::RoutesTo);
        }
    }

    fn view_node_for_reference(
        &self,
        graph: &mut GraphBuilder,
        reference: &ViewReference,
        view_nodes: &mut HashMap<String, NodeIndex>,
    ) -> NodeIndex {
        if let Some(view) = view_nodes.get(&reference.value) {
            return *view;
        }

        let node = if let Some(view_index) = self.views_by_python_name.get(&reference.value) {
            let view = &self.views[*view_index];
            let node = graph.add_node(
                view.node_key(),
                view.qualified_name.clone(),
                Some(view.module_path.clone()),
            );

            if let Some(source_file) = view.source_file {
                graph.add_edge(source_file, node, EdgeType::Contains);
            }

            node
        } else {
            graph.add_node(
                NodeKey::new(NodeType::View, reference.value.clone()),
                class_name(&reference.value).to_owned(),
                None,
            )
        };

        view_nodes.insert(reference.value.clone(), node);
        node
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolutionState {
    Unvisited,
    Visiting,
    Resolved(bool),
}

fn collect_definitions_in_suite(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    import_index: &ImportIndex,
    suite: &ast::Suite,
    class_stack: &mut Vec<String>,
) {
    for statement in suite {
        match statement {
            Stmt::ClassDef(class_def) => collect_class(
                index,
                source_file,
                module_path,
                python_module,
                import_index,
                class_def,
                class_stack,
            ),
            Stmt::FunctionDef(function_def) if class_stack.is_empty() => {
                collect_function(index, source_file, module_path, python_module, function_def)
            }
            Stmt::AsyncFunctionDef(function_def) if class_stack.is_empty() => {
                collect_async_function(index, source_file, module_path, python_module, function_def)
            }
            _ => {}
        }
    }
}

fn collect_class(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    import_index: &ImportIndex,
    class_def: &StmtClassDef,
    class_stack: &mut Vec<String>,
) {
    class_stack.push(class_def.name.to_string());

    let qualified_name = class_stack.join(".");
    let python_qualified_name = format!("{python_module}.{qualified_name}");
    let bases = class_def
        .bases
        .iter()
        .filter_map(|base| expr_model_reference(base, import_index))
        .collect();
    let relationships = collect_relationships(&class_def.body, import_index);

    index.add_class(ClassSymbol {
        source_file,
        module_path: module_path.to_owned(),
        python_module: python_module.to_owned(),
        qualified_name: qualified_name.clone(),
        python_qualified_name: python_qualified_name.clone(),
        bases,
        relationships,
        app: infer_django_app(module_path),
    });

    index.add_view(ViewSymbol {
        source_file,
        module_path: module_path.to_owned(),
        qualified_name: qualified_name.clone(),
        python_qualified_name,
    });

    collect_definitions_in_suite(
        index,
        source_file,
        module_path,
        python_module,
        import_index,
        &class_def.body,
        class_stack,
    );

    class_stack.pop();
}

fn collect_function(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    function_def: &StmtFunctionDef,
) {
    collect_view_function(
        index,
        source_file,
        module_path,
        python_module,
        function_def.name.as_str(),
    );
}

fn collect_async_function(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    function_def: &StmtAsyncFunctionDef,
) {
    collect_view_function(
        index,
        source_file,
        module_path,
        python_module,
        function_def.name.as_str(),
    );
}

fn collect_view_function(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    name: &str,
) {
    index.add_view(ViewSymbol {
        source_file,
        module_path: module_path.to_owned(),
        qualified_name: name.to_owned(),
        python_qualified_name: format!("{python_module}.{name}"),
    });
}

fn collect_routes_in_suite(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    import_index: &ImportIndex,
    suite: &ast::Suite,
) {
    for statement in suite {
        if let Some((variable, registration)) =
            router_registration_from_statement(statement, python_module, import_index)
        {
            index.add_router_registration(python_module, &variable, registration);
        }

        collect_raw_routes_from_statement(
            index,
            source_file,
            module_path,
            python_module,
            import_index,
            statement,
        );
    }
}

fn router_registration_from_statement(
    statement: &Stmt,
    python_module: &str,
    import_index: &ImportIndex,
) -> Option<(String, RouterRegistration)> {
    let Stmt::Expr(statement) = statement else {
        return None;
    };
    let Expr::Call(call) = statement.value.as_ref() else {
        return None;
    };
    let Expr::Attribute(function) = call.func.as_ref() else {
        return None;
    };

    if function.attr.as_str() != "register" {
        return None;
    }

    let variable = expr_dotted_name(&function.value)?;

    if variable.contains('.') {
        return None;
    }

    let route = call
        .args
        .first()
        .or_else(|| keyword_value(&call.keywords, "prefix"))
        .and_then(string_constant)?;
    let view_expr = call
        .args
        .get(1)
        .or_else(|| keyword_value(&call.keywords, "viewset"))?;
    let view = view_reference_from_expr(view_expr, python_module, import_index)?;

    Some((
        variable,
        RouterRegistration {
            route: normalize_router_route(&route),
            view,
        },
    ))
}

fn collect_raw_routes_from_statement(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    import_index: &ImportIndex,
    statement: &Stmt,
) {
    match statement {
        Stmt::Assign(assign) if assign.targets.iter().any(is_urlpatterns_target) => {
            collect_raw_routes_from_expr(
                index,
                source_file,
                module_path,
                python_module,
                import_index,
                &assign.value,
            );
        }
        Stmt::AugAssign(assign) if is_urlpatterns_target(&assign.target) => {
            collect_raw_routes_from_expr(
                index,
                source_file,
                module_path,
                python_module,
                import_index,
                &assign.value,
            );
        }
        _ => {}
    }
}

fn collect_raw_routes_from_expr(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    import_index: &ImportIndex,
    expr: &Expr,
) {
    match expr {
        Expr::List(list) => {
            for item in &list.elts {
                collect_raw_routes_from_expr(
                    index,
                    source_file,
                    module_path,
                    python_module,
                    import_index,
                    item,
                );
            }
        }
        Expr::Tuple(tuple) => {
            for item in &tuple.elts {
                collect_raw_routes_from_expr(
                    index,
                    source_file,
                    module_path,
                    python_module,
                    import_index,
                    item,
                );
            }
        }
        Expr::Call(call) => {
            if let Some((route, target)) = route_call(call, python_module, import_index) {
                index.add_raw_route(source_file, module_path, python_module, route, target);
            }
        }
        Expr::Attribute(_) => {
            if let Some(include) = include_reference_from_expr(expr, python_module, import_index) {
                index.add_raw_route(
                    source_file,
                    module_path,
                    python_module,
                    String::new(),
                    RawRouteTarget::Include(include),
                );
            }
        }
        _ => {}
    }
}

fn route_call(
    call: &ast::ExprCall,
    python_module: &str,
    import_index: &ImportIndex,
) -> Option<(String, RawRouteTarget)> {
    let function_name = expr_dotted_name(&call.func)?;
    let function_name = import_index.resolve(&function_name);

    if !is_django_route_function(&function_name) {
        return None;
    }

    let route = call
        .args
        .first()
        .or_else(|| keyword_value(&call.keywords, "route"))
        .and_then(string_constant)?;
    let target_expr = call
        .args
        .get(1)
        .or_else(|| keyword_value(&call.keywords, "view"))?;
    let target = route_target_from_expr(target_expr, python_module, import_index)?;

    Some((route, target))
}

fn route_target_from_expr(
    expr: &Expr,
    python_module: &str,
    import_index: &ImportIndex,
) -> Option<RawRouteTarget> {
    if let Expr::Call(call) = expr
        && let Some(include) = include_reference_from_call(call, python_module, import_index)
    {
        return Some(RawRouteTarget::Include(include));
    }

    if matches!(expr, Expr::Attribute(attribute) if attribute.attr.as_str() == "urls")
        && let Some(include) = include_reference_from_expr(expr, python_module, import_index)
    {
        return Some(RawRouteTarget::Include(include));
    }

    view_reference_from_expr(expr, python_module, import_index).map(RawRouteTarget::View)
}

fn include_reference_from_call(
    call: &ast::ExprCall,
    python_module: &str,
    import_index: &ImportIndex,
) -> Option<IncludeReference> {
    let function_name = expr_dotted_name(&call.func)?;
    let function_name = import_index.resolve(&function_name);

    if !is_django_include_function(&function_name) {
        return None;
    }

    let target_expr = call
        .args
        .first()
        .or_else(|| keyword_value(&call.keywords, "module"))?;

    include_reference_from_expr(target_expr, python_module, import_index)
}

fn include_reference_from_expr(
    expr: &Expr,
    python_module: &str,
    import_index: &ImportIndex,
) -> Option<IncludeReference> {
    match expr {
        Expr::Constant(_) => string_constant(expr).map(IncludeReference::Module),
        Expr::Tuple(tuple) => tuple
            .elts
            .first()
            .and_then(|expr| include_reference_from_expr(expr, python_module, import_index)),
        Expr::Attribute(attribute) if attribute.attr.as_str() == "urls" => {
            let full_name = expr_dotted_name(expr)?;
            let base_name = expr_dotted_name(&attribute.value)?;
            let first_segment = first_segment(&base_name);

            if !base_name.contains('.') && !import_index.has_alias(first_segment) {
                Some(IncludeReference::Router {
                    python_module: python_module.to_owned(),
                    variable: base_name,
                })
            } else {
                Some(IncludeReference::Module(import_index.resolve(&full_name)))
            }
        }
        _ => {
            let dotted_name = expr_dotted_name(expr)?;
            let resolved_name = import_index.resolve(&dotted_name);

            (resolved_name != dotted_name).then_some(IncludeReference::Module(resolved_name))
        }
    }
}

fn view_reference_from_expr(
    expr: &Expr,
    python_module: &str,
    import_index: &ImportIndex,
) -> Option<ViewReference> {
    let dotted_name = if let Expr::Call(call) = expr {
        class_based_view_name(call).or_else(|| expr_dotted_name(expr))?
    } else {
        expr_dotted_name(expr)?
    };
    let resolved_name = import_index.resolve(&dotted_name);
    let value = if resolved_name == dotted_name && !dotted_name.contains('.') {
        format!("{python_module}.{dotted_name}")
    } else {
        resolved_name
    };

    Some(ViewReference::new(value))
}

fn class_based_view_name(call: &ast::ExprCall) -> Option<String> {
    let Expr::Attribute(function) = call.func.as_ref() else {
        return None;
    };

    if function.attr.as_str() == "as_view" {
        expr_dotted_name(&function.value)
    } else {
        None
    }
}

fn is_urlpatterns_target(expr: &Expr) -> bool {
    matches!(expr, Expr::Name(name) if name.id.as_str() == "urlpatterns")
}

fn is_django_route_function(value: &str) -> bool {
    matches!(
        value,
        "django.urls.path"
            | "django.urls.re_path"
            | "django.conf.urls.url"
            | "path"
            | "re_path"
            | "url"
    )
}

fn is_django_include_function(value: &str) -> bool {
    matches!(value, "django.urls.include" | "include")
}

fn keyword_value<'a>(keywords: &'a [ast::Keyword], name: &str) -> Option<&'a Expr> {
    keywords.iter().find_map(|keyword| {
        keyword
            .arg
            .as_ref()
            .is_some_and(|arg| arg.as_str() == name)
            .then_some(&keyword.value)
    })
}

fn string_constant(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Constant(constant) => match &constant.value {
            Constant::Str(value) => Some(value.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn normalize_router_route(route: &str) -> String {
    if route.is_empty() || route.ends_with('/') {
        route.to_owned()
    } else {
        format!("{route}/")
    }
}

fn combine_route_patterns(prefix: &str, route: &str) -> String {
    match (prefix.is_empty(), route.is_empty()) {
        (true, _) => route.to_owned(),
        (_, true) => prefix.to_owned(),
        _ => format!("{prefix}{route}"),
    }
}

fn collect_relationships(suite: &ast::Suite, import_index: &ImportIndex) -> Vec<ModelRelationship> {
    let mut relationships = Vec::new();

    for statement in suite {
        match statement {
            Stmt::Assign(assign) => {
                if let Some(relationship) = relationship_from_expr(&assign.value, import_index) {
                    relationships.push(relationship);
                }
            }
            Stmt::AnnAssign(assign) => {
                if let Some(value) = &assign.value
                    && let Some(relationship) = relationship_from_expr(value, import_index)
                {
                    relationships.push(relationship);
                }
            }
            _ => {}
        }
    }

    relationships
}

fn relationship_from_expr(expr: &Expr, import_index: &ImportIndex) -> Option<ModelRelationship> {
    let Expr::Call(call) = expr else {
        return None;
    };

    let field_type = expr_dotted_name(&call.func)?;
    let resolved_field_type = import_index.resolve(&field_type);

    if !is_django_relationship_field(&resolved_field_type) {
        return None;
    }

    let target_expr = call
        .args
        .first()
        .or_else(|| call.keywords.iter().find_map(keyword_to_arg_expr))?;
    let target = expr_model_reference(target_expr, import_index)?;

    Some(ModelRelationship { target })
}

fn keyword_to_arg_expr(keyword: &ast::Keyword) -> Option<&Expr> {
    if keyword.arg.as_ref().is_some_and(|arg| arg.as_str() == "to") {
        Some(&keyword.value)
    } else {
        None
    }
}

fn is_django_relationship_field(value: &str) -> bool {
    matches!(
        value,
        "django.db.models.ForeignKey"
            | "django.db.models.OneToOneField"
            | "django.db.models.ManyToManyField"
    )
}

fn expr_model_reference(expr: &Expr, import_index: &ImportIndex) -> Option<ModelReference> {
    match expr {
        Expr::Constant(constant) => match &constant.value {
            Constant::Str(value) => Some(ModelReference::new(value.clone())),
            _ => None,
        },
        _ => expr_dotted_name(expr).map(|name| ModelReference::new(import_index.resolve(&name))),
    }
}

fn infer_django_app(module_path: &str) -> Option<DjangoApp> {
    let path = Path::new(module_path);
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    let app_components = if module_path.ends_with("models.py") {
        components
            .get(..components.len().saturating_sub(1))
            .unwrap_or_default()
    } else if let Some(models_position) = components
        .iter()
        .position(|component| component == "models")
    {
        components.get(..models_position).unwrap_or_default()
    } else {
        return None;
    };

    let label = app_components.last()?.clone();
    let path = app_components.join("/");
    let identifier = path.replace('/', ".");

    Some(DjangoApp {
        identifier,
        label,
        path,
    })
}

fn class_name(qualified_name: &str) -> &str {
    qualified_name
        .rsplit_once('.')
        .map_or(qualified_name, |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analysis::analyze_python_project, parsing::parse_python_files};
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
            let path = std::env::temp_dir()
                .join(format!("spelunking-{name}-{}-{stamp}", std::process::id()));

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
    fn discovers_models_inheriting_from_django_models_alias() {
        let project = TempProject::new("django-models-alias");
        let models = project.write(
            "products/models.py",
            r#"
from django.db import models

class Product(models.Model):
    pass

class Helper:
    pass
"#,
        );
        let report = parse_python_files(std::slice::from_ref(&models));

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[models], &report.modules);
        let model_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.node_type == NodeType::Model)
            .collect::<Vec<_>>();

        assert_eq!(model_nodes.len(), 1);
        assert_eq!(model_nodes[0].id, "model:products/models.py:Product");
        assert_eq!(model_nodes[0].label, "Product");
        assert_eq!(model_nodes[0].path.as_deref(), Some("products/models.py"));
        assert_eq!(graph.node_count_by_type(NodeType::App), 1);
        assert_eq!(graph.edge_count_by_type(EdgeType::Contains), 2);
    }

    #[test]
    fn discovers_models_inheriting_from_project_base_models() {
        let project = TempProject::new("django-models-inheritance");
        let base = project.write(
            "commerce/models/base.py",
            r#"
from django.db import models

class BaseModel(models.Model):
    pass
"#,
        );
        let product = project.write(
            "commerce/models/product.py",
            r#"
from .base import BaseModel

class Product(BaseModel):
    pass
"#,
        );
        let report = parse_python_files(&[base.clone(), product.clone()]);

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[base, product], &report.modules);

        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "model:commerce/models/product.py:Product")
        );
        assert_eq!(graph.node_count_by_type(NodeType::Model), 2);
        assert_eq!(graph.edge_count_by_type(EdgeType::Inherits), 1);
    }

    #[test]
    fn discovers_model_relationships() {
        let project = TempProject::new("django-models-relationships");
        let models_file = project.write(
            "shop/models.py",
            r#"
from django.db import models

class Category(models.Model):
    pass

class Product(models.Model):
    category = models.ForeignKey(Category, on_delete=models.CASCADE)
    related = models.ManyToManyField("self")
"#,
        );
        let report = parse_python_files(std::slice::from_ref(&models_file));

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[models_file], &report.modules);

        assert_eq!(graph.node_count_by_type(NodeType::Model), 2);
        assert_eq!(graph.edge_count_by_type(EdgeType::RelatesTo), 2);
    }

    #[test]
    fn resolves_string_relationship_targets_by_app_label() {
        let project = TempProject::new("django-models-string-relationships");
        let category = project.write(
            "catalog/models.py",
            r#"
from django.db import models

class Category(models.Model):
    pass
"#,
        );
        let product = project.write(
            "shop/models.py",
            r#"
from django.db import models

class Product(models.Model):
    category = models.ForeignKey("catalog.Category", on_delete=models.CASCADE)
"#,
        );
        let report = parse_python_files(&[category.clone(), product.clone()]);

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[category, product], &report.modules);

        assert_eq!(graph.node_count_by_type(NodeType::Model), 2);
        assert_eq!(graph.edge_count_by_type(EdgeType::RelatesTo), 1);
    }

    #[test]
    fn discovers_models_inheriting_from_imported_model_symbol() {
        let project = TempProject::new("django-models-symbol");
        let models = project.write(
            "orders/models.py",
            r#"
from django.db.models import Model

class Order(Model):
    pass
"#,
        );
        let report = parse_python_files(std::slice::from_ref(&models));

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[models], &report.modules);

        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "model:orders/models.py:Order")
        );
    }

    #[test]
    fn discovers_models_inheriting_from_import_alias() {
        let project = TempProject::new("django-models-import-alias");
        let models = project.write(
            "customers/models.py",
            r#"
import django.db.models as django_models

class Customer(django_models.Model):
    pass
"#,
        );
        let report = parse_python_files(std::slice::from_ref(&models));

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[models], &report.modules);

        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "model:customers/models.py:Customer")
        );
    }

    #[test]
    fn does_not_treat_unresolved_model_base_as_django_model() {
        let project = TempProject::new("django-models-unresolved");
        let models = project.write(
            "plain/models.py",
            r#"
class Local(Model):
    pass
"#,
        );
        let report = parse_python_files(std::slice::from_ref(&models));

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[models], &report.modules);

        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.node_type != NodeType::Model)
        );
    }

    #[test]
    fn does_not_treat_unresolved_models_attribute_as_django_model() {
        let project = TempProject::new("django-models-attribute-unresolved");
        let models = project.write(
            "plain/models.py",
            r#"
class Local(models.Model):
    pass
"#,
        );
        let report = parse_python_files(std::slice::from_ref(&models));

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[models], &report.modules);

        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.node_type != NodeType::Model)
        );
    }

    #[test]
    fn discovers_models_inheriting_from_full_django_import() {
        let project = TempProject::new("django-models-full-import");
        let models = project.write(
            "inventory/models.py",
            r#"
import django.db.models

class StockItem(django.db.models.Model):
    pass
"#,
        );
        let report = parse_python_files(std::slice::from_ref(&models));

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[models], &report.modules);

        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "model:inventory/models.py:StockItem")
        );
    }

    #[test]
    fn maps_url_patterns_to_function_and_class_views() {
        let project = TempProject::new("django-url-patterns");
        let views = project.write(
            "shop/views.py",
            r#"
def product_list(request):
    pass

class ProductDetailView:
    pass
"#,
        );
        let urls = project.write(
            "shop/urls.py",
            r#"
from django.urls import path, re_path
from . import views
from .views import ProductDetailView

urlpatterns = [
    path("products/", views.product_list, name="products"),
    re_path(r"^products/(?P<pk>\d+)/$", ProductDetailView.as_view(), name="product-detail"),
]
"#,
        );
        let report = parse_python_files(&[views.clone(), urls.clone()]);

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[views, urls], &report.modules);

        assert_eq!(graph.node_count_by_type(NodeType::Url), 2);
        assert_eq!(graph.node_count_by_type(NodeType::View), 2);
        assert_eq!(graph.edge_count_by_type(EdgeType::RoutesTo), 2);
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "view:shop.views.product_list"
                    && node.path.as_deref() == Some("shop/views.py"))
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "view:shop.views.ProductDetailView"
                    && node.path.as_deref() == Some("shop/views.py"))
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.node_type == NodeType::Url && node.label == "products/")
        );
        assert!(graph.nodes.iter().any(
            |node| node.node_type == NodeType::Url && node.label == "^products/(?P<pk>\\d+)/$"
        ));
    }

    #[test]
    fn expands_basic_include_patterns() {
        let project = TempProject::new("django-url-includes");
        let root_urls = project.write(
            "project/urls.py",
            r#"
from django.urls import include, path

urlpatterns = [
    path("shop/", include("shop.urls")),
]
"#,
        );
        let shop_urls = project.write(
            "shop/urls.py",
            r#"
from django.urls import path
from .views import product_list

urlpatterns = [
    path("products/", product_list, name="products"),
]
"#,
        );
        let views = project.write(
            "shop/views.py",
            r#"
def product_list(request):
    pass
"#,
        );
        let files = vec![root_urls, shop_urls, views];
        let report = parse_python_files(&files);

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &files, &report.modules);

        assert_eq!(graph.node_count_by_type(NodeType::Url), 1);
        assert_eq!(graph.edge_count_by_type(EdgeType::RoutesTo), 1);
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.node_type == NodeType::Url && node.label == "shop/products/")
        );
    }

    #[test]
    fn maps_included_drf_router_registrations_to_viewsets() {
        let project = TempProject::new("django-router-registrations");
        let urls = project.write(
            "api/urls.py",
            r#"
from django.urls import include, path
from rest_framework.routers import DefaultRouter
from .views import ProductViewSet

router = DefaultRouter()
router.register("products", ProductViewSet, basename="product")

urlpatterns = [
    path("api/", include(router.urls)),
]
"#,
        );
        let views = project.write(
            "api/views.py",
            r#"
class ProductViewSet:
    pass
"#,
        );
        let report = parse_python_files(&[urls.clone(), views.clone()]);

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[urls, views], &report.modules);

        assert_eq!(graph.node_count_by_type(NodeType::Url), 1);
        assert_eq!(graph.node_count_by_type(NodeType::View), 1);
        assert_eq!(graph.edge_count_by_type(EdgeType::RoutesTo), 1);
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.node_type == NodeType::Url && node.label == "api/products/")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "view:api.views.ProductViewSet"
                    && node.path.as_deref() == Some("api/views.py"))
        );
    }

    #[test]
    fn maps_direct_drf_router_urlpatterns_to_viewsets() {
        let project = TempProject::new("django-direct-router-registrations");
        let urls = project.write(
            "api/urls.py",
            r#"
from rest_framework.routers import DefaultRouter
from .views import ProductViewSet

router = DefaultRouter()
router.register("products", ProductViewSet, basename="product")

urlpatterns = router.urls
"#,
        );
        let views = project.write(
            "api/views.py",
            r#"
class ProductViewSet:
    pass
"#,
        );
        let report = parse_python_files(&[urls.clone(), views.clone()]);

        assert!(!report.has_diagnostics());

        let graph = analyze_python_project(&project.path, &[urls, views], &report.modules);

        assert_eq!(graph.node_count_by_type(NodeType::Url), 1);
        assert_eq!(graph.edge_count_by_type(EdgeType::RoutesTo), 1);
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.node_type == NodeType::Url && node.label == "products/")
        );
    }
}

use crate::{
    analysis::{AnalysisContext, Analyzer, SourceLanguage},
    graph::{EdgeType, GraphBuilder, NodeKey, NodeType, relative_path_identifier},
};
use petgraph::graph::NodeIndex;
use rustpython_parser::ast::{self, Constant, Expr, Stmt, StmtClassDef};
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

#[derive(Debug, Default)]
struct DjangoProjectIndex {
    classes: Vec<ClassSymbol>,
    classes_by_python_name: HashMap<String, usize>,
    classes_by_module_local_name: HashMap<(String, String), usize>,
    classes_by_app_and_name: HashMap<(String, String), usize>,
    model_class_indices: HashSet<usize>,
}

impl DjangoProjectIndex {
    fn from_context(context: &AnalysisContext<'_>) -> Self {
        let mut index = Self::default();

        for module in context.python_modules() {
            let module_path = relative_path_identifier(context.root(), &module.path);
            let python_module = python_module_path(&module_path);
            let import_index = ImportIndex::from_suite(&module.ast, &python_module);
            let source_file = context.source_file_index(&module.path);

            collect_classes_in_suite(
                &mut index,
                source_file,
                &module_path,
                &python_module,
                &import_index,
                &module.ast,
                &mut Vec::new(),
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolutionState {
    Unvisited,
    Visiting,
    Resolved(bool),
}

fn collect_classes_in_suite(
    index: &mut DjangoProjectIndex,
    source_file: Option<NodeIndex>,
    module_path: &str,
    python_module: &str,
    import_index: &ImportIndex,
    suite: &ast::Suite,
    class_stack: &mut Vec<String>,
) {
    for statement in suite {
        if let Stmt::ClassDef(class_def) = statement {
            collect_class(
                index,
                source_file,
                module_path,
                python_module,
                import_index,
                class_def,
                class_stack,
            );
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
        qualified_name,
        python_qualified_name,
        bases,
        relationships,
        app: infer_django_app(module_path),
    });

    collect_classes_in_suite(
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

fn expr_dotted_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        Expr::Attribute(attribute) => {
            let parent = expr_dotted_name(&attribute.value)?;

            Some(format!("{parent}.{}", attribute.attr))
        }
        _ => None,
    }
}

#[derive(Debug, Default)]
struct ImportIndex {
    aliases: HashMap<String, String>,
}

impl ImportIndex {
    fn from_suite(suite: &ast::Suite, python_module: &str) -> Self {
        let mut index = Self::default();

        for statement in suite {
            match statement {
                Stmt::Import(import) => index.add_imports(&import.names),
                Stmt::ImportFrom(import_from) => {
                    let module = import_from_module(
                        python_module,
                        import_from.level,
                        import_from.module.as_ref(),
                    );

                    index.add_import_from(module.as_deref(), &import_from.names);
                }
                _ => {}
            }
        }

        index
    }

    fn add_imports(&mut self, aliases: &[ast::Alias]) {
        for alias in aliases {
            let imported_name = alias.name.to_string();
            let (local_name, resolved_name) = if let Some(asname) = alias.asname.as_ref() {
                (asname.to_string(), imported_name)
            } else {
                let local_name = first_segment(&imported_name).to_owned();

                (local_name.clone(), local_name)
            };

            self.aliases.insert(local_name, resolved_name);
        }
    }

    fn add_import_from(&mut self, module: Option<&str>, aliases: &[ast::Alias]) {
        let Some(module) = module else {
            return;
        };

        for alias in aliases {
            let imported_name = alias.name.to_string();
            let local_name = alias
                .asname
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| imported_name.clone());
            let fully_qualified_name = format!("{module}.{imported_name}");

            self.aliases.insert(local_name, fully_qualified_name);
        }
    }

    fn resolve(&self, dotted_name: &str) -> String {
        let Some((head, tail)) = dotted_name.split_once('.') else {
            return self
                .aliases
                .get(dotted_name)
                .cloned()
                .unwrap_or_else(|| dotted_name.to_owned());
        };

        if let Some(resolved_head) = self.aliases.get(head) {
            format!("{resolved_head}.{tail}")
        } else {
            dotted_name.to_owned()
        }
    }
}

fn import_from_module(
    python_module: &str,
    level: Option<ast::Int>,
    module: Option<&ast::Identifier>,
) -> Option<String> {
    let level = level.map_or(0, |level| level.to_usize());

    if level == 0 {
        return module.map(ToString::to_string);
    }

    let mut segments = python_module.split('.').collect::<Vec<_>>();

    segments.pop();

    for _ in 1..level {
        segments.pop()?;
    }

    if let Some(module) = module {
        segments.extend(module.as_str().split('.'));
    }

    if segments.is_empty() {
        None
    } else {
        Some(segments.join("."))
    }
}

fn python_module_path(module_path: &str) -> String {
    let without_extension = module_path.strip_suffix(".py").unwrap_or(module_path);
    let without_init = without_extension
        .strip_suffix("/__init__")
        .unwrap_or(without_extension);

    without_init.replace('/', ".")
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

fn first_segment(value: &str) -> &str {
    value.split_once('.').map_or(value, |(first, _)| first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis::analyze_python_project,
        parsing::{ParsedPythonModule, parse_python_files},
    };
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
    fn import_index_resolves_common_django_model_imports() {
        let project = TempProject::new("django-import-index");
        let models = project.write(
            "models.py",
            r#"
from django.db import models
from django.db.models import Model as BaseModel
import django.db.models
import django.db.models as django_models
"#,
        );
        let ParsedPythonModule { ast, .. } = parse_python_files(std::slice::from_ref(&models))
            .modules
            .remove(0);
        let index = ImportIndex::from_suite(&ast, "models");

        assert_eq!(index.resolve("models.Model"), "django.db.models.Model");
        assert_eq!(index.resolve("BaseModel"), "django.db.models.Model");
        assert_eq!(
            index.resolve("django_models.Model"),
            "django.db.models.Model"
        );
        assert_eq!(
            index.resolve("django.db.models.Model"),
            "django.db.models.Model"
        );
    }

    #[test]
    fn import_index_resolves_relative_imports() {
        let project = TempProject::new("django-relative-import-index");
        let models = project.write(
            "commerce/models/product.py",
            r#"
from .base import BaseModel
from ..shared import TimestampedModel
"#,
        );
        let ParsedPythonModule { ast, .. } = parse_python_files(std::slice::from_ref(&models))
            .modules
            .remove(0);
        let index = ImportIndex::from_suite(&ast, "commerce.models.product");

        assert_eq!(index.resolve("BaseModel"), "commerce.models.base.BaseModel");
        assert_eq!(
            index.resolve("TimestampedModel"),
            "commerce.shared.TimestampedModel"
        );
    }
}

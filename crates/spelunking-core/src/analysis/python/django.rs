use crate::{
    analysis::{AnalysisContext, Analyzer, SourceLanguage},
    graph::{EdgeType, GraphBuilder, NodeKey, NodeType, relative_path_identifier},
};
use rustpython_parser::ast::{self, Expr, Stmt, StmtClassDef};
use std::collections::HashMap;

pub struct DjangoModelAnalyzer;

impl Analyzer for DjangoModelAnalyzer {
    fn name(&self) -> &'static str {
        "python.django.models"
    }

    fn language(&self) -> SourceLanguage {
        SourceLanguage::Python
    }

    fn analyze(&self, context: &AnalysisContext<'_>, graph: &mut GraphBuilder) {
        for module in context.python_modules() {
            let import_index = ImportIndex::from_suite(&module.ast);
            let source_file = context.source_file_index(&module.path);
            let module_path = relative_path_identifier(context.root(), &module.path);

            discover_models_in_suite(
                graph,
                source_file,
                &module_path,
                &import_index,
                &module.ast,
                &mut Vec::new(),
            );
        }
    }
}

fn discover_models_in_suite(
    graph: &mut GraphBuilder,
    source_file: Option<petgraph::graph::NodeIndex>,
    module_path: &str,
    import_index: &ImportIndex,
    suite: &ast::Suite,
    class_stack: &mut Vec<String>,
) {
    for statement in suite {
        if let Stmt::ClassDef(class_def) = statement {
            discover_model_class(
                graph,
                source_file,
                module_path,
                import_index,
                class_def,
                class_stack,
            );
        }
    }
}

fn discover_model_class(
    graph: &mut GraphBuilder,
    source_file: Option<petgraph::graph::NodeIndex>,
    module_path: &str,
    import_index: &ImportIndex,
    class_def: &StmtClassDef,
    class_stack: &mut Vec<String>,
) {
    class_stack.push(class_def.name.to_string());

    if class_def
        .bases
        .iter()
        .any(|base| is_django_model_base(base, import_index))
    {
        let qualified_name = class_stack.join(".");
        let model_identifier = format!("{module_path}:{qualified_name}");
        let model = graph.add_node(
            NodeKey::new(NodeType::Model, model_identifier),
            qualified_name,
            Some(module_path.to_owned()),
        );

        if let Some(source_file) = source_file {
            graph.add_edge(source_file, model, EdgeType::Contains);
        }
    }

    discover_models_in_suite(
        graph,
        source_file,
        module_path,
        import_index,
        &class_def.body,
        class_stack,
    );
    class_stack.pop();
}

fn is_django_model_base(base: &Expr, import_index: &ImportIndex) -> bool {
    let Some(base_name) = dotted_expr_name(base) else {
        return false;
    };
    let resolved_base = import_index.resolve(&base_name);

    resolved_base == "django.db.models.Model"
}

fn dotted_expr_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        Expr::Attribute(attribute) => {
            let parent = dotted_expr_name(&attribute.value)?;

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
    fn from_suite(suite: &ast::Suite) -> Self {
        let mut index = Self::default();

        for statement in suite {
            match statement {
                Stmt::Import(import) => index.add_imports(&import.names),
                Stmt::ImportFrom(import_from) if !is_relative_import(import_from.level) => {
                    index.add_import_from(import_from.module.as_ref(), &import_from.names);
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

    fn add_import_from(&mut self, module: Option<&ast::Identifier>, aliases: &[ast::Alias]) {
        let Some(module) = module else {
            return;
        };
        let module = module.to_string();

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

fn first_segment(value: &str) -> &str {
    value.split_once('.').map_or(value, |(first, _)| first)
}

fn is_relative_import(level: Option<ast::Int>) -> bool {
    level.is_some_and(|level| level.to_u32() > 0)
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
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].edge_type, EdgeType::Contains);
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
        let index = ImportIndex::from_suite(&ast);

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
}

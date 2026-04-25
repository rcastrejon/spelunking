use rustpython_parser::ast::{self, Expr};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub(crate) struct ImportIndex {
    aliases: HashMap<String, String>,
}

impl ImportIndex {
    pub(crate) fn from_suite(suite: &ast::Suite, python_module: &str) -> Self {
        let mut index = Self::default();

        for statement in suite {
            match statement {
                ast::Stmt::Import(import) => index.add_imports(&import.names),
                ast::Stmt::ImportFrom(import_from) => {
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

    pub(crate) fn resolve(&self, dotted_name: &str) -> String {
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

    pub(crate) fn has_alias(&self, name: &str) -> bool {
        self.aliases.contains_key(name)
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
}

pub(crate) fn expr_dotted_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(name) => Some(name.id.to_string()),
        Expr::Attribute(attribute) => {
            let parent = expr_dotted_name(&attribute.value)?;

            Some(format!("{parent}.{}", attribute.attr))
        }
        _ => None,
    }
}

pub(crate) fn python_module_path(module_path: &str) -> String {
    let without_extension = module_path.strip_suffix(".py").unwrap_or(module_path);
    let without_init = without_extension
        .strip_suffix("/__init__")
        .unwrap_or(without_extension);

    without_init.replace('/', ".")
}

pub(crate) fn first_segment(value: &str) -> &str {
    value.split_once('.').map_or(value, |(first, _)| first)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsing::{ParsedPythonModule, parse_python_files};
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

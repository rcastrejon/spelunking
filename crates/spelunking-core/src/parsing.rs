use rayon::prelude::*;
use rustpython_parser::{Parse, ast};
use std::{fs, path::PathBuf};

#[derive(Debug)]
pub struct ParsedPythonModule {
    pub path: PathBuf,
    pub source: String,
    pub ast: ast::Suite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonParseDiagnostic {
    pub path: PathBuf,
    pub kind: PythonParseDiagnosticKind,
    pub message: String,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonParseDiagnosticKind {
    Read,
    Syntax,
}

impl PythonParseDiagnosticKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Syntax => "syntax",
        }
    }
}

#[derive(Debug, Default)]
pub struct PythonParseReport {
    pub modules: Vec<ParsedPythonModule>,
    pub diagnostics: Vec<PythonParseDiagnostic>,
}

impl PythonParseReport {
    pub fn parsed_count(&self) -> usize {
        self.modules.len()
    }

    pub fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
    }

    pub fn has_diagnostics(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

pub fn parse_python_files(paths: &[PathBuf]) -> PythonParseReport {
    let outcomes = paths
        .par_iter()
        .map(|path| parse_python_file(path.clone()))
        .collect::<Vec<_>>();

    let mut report = PythonParseReport::default();

    for outcome in outcomes {
        match outcome {
            PythonParseOutcome::Parsed(module) => report.modules.push(module),
            PythonParseOutcome::Diagnostic(diagnostic) => report.diagnostics.push(diagnostic),
        }
    }

    report
}

enum PythonParseOutcome {
    Parsed(ParsedPythonModule),
    Diagnostic(PythonParseDiagnostic),
}

fn parse_python_file(path: PathBuf) -> PythonParseOutcome {
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            return PythonParseOutcome::Diagnostic(PythonParseDiagnostic {
                path,
                kind: PythonParseDiagnosticKind::Read,
                message: error.to_string(),
                offset: None,
            });
        }
    };

    let source_path = path.to_string_lossy();

    match ast::Suite::parse(&source, &source_path) {
        Ok(ast) => PythonParseOutcome::Parsed(ParsedPythonModule { path, source, ast }),
        Err(error) => PythonParseOutcome::Diagnostic(PythonParseDiagnostic {
            path,
            kind: PythonParseDiagnosticKind::Syntax,
            message: error.to_string(),
            offset: Some(u32::from(error.offset)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
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
    fn parses_valid_python_modules() {
        let project = TempProject::new("parse-valid");
        let path = project.write("models.py", "class Product:\n    pass\n");

        let report = parse_python_files(std::slice::from_ref(&path));

        assert_eq!(report.parsed_count(), 1);
        assert_eq!(report.diagnostic_count(), 0);
        assert_eq!(report.modules[0].path, path);
        assert_eq!(report.modules[0].ast.len(), 1);
    }

    #[test]
    fn reports_syntax_diagnostics_without_stopping_the_batch() {
        let project = TempProject::new("parse-diagnostics");
        let valid = project.write("valid.py", "price = 1\n");
        let invalid = project.write("invalid.py", "def broken(:\n    pass\n");

        let report = parse_python_files(&[valid, invalid.clone()]);

        assert_eq!(report.parsed_count(), 1);
        assert_eq!(report.diagnostic_count(), 1);
        assert_eq!(report.diagnostics[0].path, invalid);
        assert_eq!(
            report.diagnostics[0].kind,
            PythonParseDiagnosticKind::Syntax
        );
        assert!(report.diagnostics[0].offset.is_some());
    }

    #[test]
    fn reports_read_diagnostics_without_stopping_the_batch() {
        let project = TempProject::new("parse-read");
        let valid = project.write("valid.py", "price = 1\n");
        let missing = project.path.join("missing.py");

        let report = parse_python_files(&[valid, missing.clone()]);

        assert_eq!(report.parsed_count(), 1);
        assert_eq!(report.diagnostic_count(), 1);
        assert_eq!(report.diagnostics[0].path, missing);
        assert_eq!(report.diagnostics[0].kind, PythonParseDiagnosticKind::Read);
        assert_eq!(report.diagnostics[0].offset, None);
    }
}

use ignore::WalkBuilder;
use std::{
    error::Error,
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum DiscoveryError {
    InvalidRoot { path: PathBuf },
    RootNotDirectory { path: PathBuf },
    Walk { source: ignore::Error },
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoot { path } => {
                write!(f, "target path does not exist: {}", path.display())
            }
            Self::RootNotDirectory { path } => {
                write!(f, "target path is not a directory: {}", path.display())
            }
            Self::Walk { source } => write!(f, "failed while walking target directory: {source}"),
        }
    }
}

impl Error for DiscoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Walk { source } => Some(source),
            Self::InvalidRoot { .. } | Self::RootNotDirectory { .. } => None,
        }
    }
}

pub fn discover_python_files(root: impl AsRef<Path>) -> Result<Vec<PathBuf>, DiscoveryError> {
    let root = root.as_ref();

    if !root.exists() {
        return Err(DiscoveryError::InvalidRoot {
            path: root.to_path_buf(),
        });
    }

    if !root.is_dir() {
        return Err(DiscoveryError::RootNotDirectory {
            path: root.to_path_buf(),
        });
    }

    let root = root
        .canonicalize()
        .map_err(|_| DiscoveryError::InvalidRoot {
            path: root.to_path_buf(),
        })?;

    let mut python_files = Vec::new();
    let walker = WalkBuilder::new(root)
        .standard_filters(true)
        .require_git(false)
        .build();

    for entry in walker {
        let entry = entry.map_err(|source| DiscoveryError::Walk { source })?;

        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
            && entry.path().extension() == Some(OsStr::new("py"))
        {
            python_files.push(entry.path().to_path_buf());
        }
    }

    python_files.sort();
    Ok(python_files)
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

        fn write(&self, path: &str, contents: &str) {
            let path = self.path.join(path);

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("parent directories should be created");
            }

            fs::write(path, contents).expect("temp project file should be written");
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn discovers_python_files_in_deterministic_order() {
        let project = TempProject::new("discovery-order");
        project.write("app/views.py", "");
        project.write("app/models.py", "");
        project.write("README.md", "");

        let files = discover_python_files(&project.path).expect("discovery should succeed");

        assert_eq!(
            files,
            vec![
                project.path.join("app/models.py").canonicalize().unwrap(),
                project.path.join("app/views.py").canonicalize().unwrap(),
            ]
        );
    }

    #[test]
    fn respects_gitignore_rules() {
        let project = TempProject::new("discovery-ignore");
        project.write(".gitignore", "ignored/\n");
        project.write("app/models.py", "");
        project.write("ignored/generated.py", "");

        let files = discover_python_files(&project.path).expect("discovery should succeed");

        assert_eq!(
            files,
            vec![project.path.join("app/models.py").canonicalize().unwrap()]
        );
    }

    #[test]
    fn rejects_non_directory_roots() {
        let project = TempProject::new("discovery-root");
        project.write("settings.py", "");

        let error = discover_python_files(project.path.join("settings.py")).unwrap_err();

        assert!(matches!(error, DiscoveryError::RootNotDirectory { .. }));
    }
}

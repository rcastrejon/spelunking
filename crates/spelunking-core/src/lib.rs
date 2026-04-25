//! Core library for Spelunking.

pub mod discovery;
pub mod parsing;

pub use discovery::{DiscoveryError, discover_python_files};
pub use parsing::{
    ParsedPythonModule, PythonParseDiagnostic, PythonParseDiagnosticKind, PythonParseReport,
    parse_python_files,
};

# Spelunking

Spelunking is planned as a Rust-based architecture mapping core for complex Django projects.

The repository is intentionally minimal at this stage. It only establishes the workspace shape for a reusable core crate and future clients.

## Workspace

- `crates/spelunking-core`: reusable analysis core
- `crates/spelunking-cli`: command-line client shell

## Current Usage

The first implementation slice discovers Python files and parses them into RustPython ASTs:

```sh
cargo run -p spelunking-cli -- /path/to/django-project
```

Use `--list-files` to print every discovered Python file. Use `--fail-on-diagnostics` to return a non-zero exit code when any file cannot be read or parsed.

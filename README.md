# Spelunking

Spelunking is planned as a Rust-based architecture mapping core for complex Django projects.

The repository is intentionally minimal at this stage. It only establishes the workspace shape for a reusable core crate and future clients.

## Workspace

- `crates/spelunking-core`: reusable analysis core
- `crates/spelunking-cli`: command-line client shell

## Current Usage

Spelunking discovers Python files, parses them into RustPython ASTs, and emits a Django architecture graph:

```sh
cargo run -p spelunking-cli -- /path/to/django-project
```

Use `--list-files` to print every discovered Python file. Use `--fail-on-diagnostics` to return a non-zero exit code when any file cannot be read or parsed.

The CLI can also emit the current graph contract as JSON. The graph contains source-file, Django app, model, URL, view, serializer, form, middleware, signal, signal handler, and task nodes, plus containment, inheritance, direct ORM relationship, URL routing, serialization, query, middleware intercept, and trigger edges:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --format json
```

Current Django analysis includes:

- model discovery through direct and inherited Django model bases
- direct ORM relationships through `ForeignKey`, `OneToOneField`, and `ManyToManyField`
- URL route discovery from `path(...)`, `re_path(...)`, and legacy `url(...)`
- function views and class-based views through `.as_view()`
- basic `include(...)` expansion
- basic DRF router registrations included through `router.urls`
- runtime context from `INSTALLED_APPS`, `ROOT_URLCONF`, and `MIDDLEWARE`
- app config labels through `django.apps.AppConfig`
- middleware-to-URL intercept edges for configured request wrappers
- signal receivers through `@receiver(...)` and `signal.connect(...)`
- Celery task discovery through `@shared_task` and `@app.task`
- trigger edges from models to signals, signals to handlers, and handlers to tasks
- DRF `Serializer` / `ModelSerializer` discovery
- Django `Form` / `ModelForm` discovery
- `Meta.model` bindings from serializers/forms to models
- view references to serializers, forms, and direct ORM model usage

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

Inspect a specific Django model field to produce a structural radiography with model location, candidate lifecycle field, detected states, fields, related models, relevant methods, related serializers/views, evidence, and confidence. Use the Django app label when a short model name may be ambiguous:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --inspect-subject reservations.Reservation.status
```

Short subjects such as `Reservation.status` are accepted only when exactly one plausible Django model named `Reservation` exists. If multiple apps define the same model name, Spelunking rejects the request and suggests app-qualified subjects like `web.Ticket.status`.

Use JSON when another tool or agent should consume the result:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --inspect-subject reservations.Reservation.status --format json
```

Inspect behavior for the same subject to find mutation sites and approximate paths such as route-to-view, serializer, task, signal, webhook, admin action, and queryset update flows:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --inspect-behavior reservations.Reservation.status
```

The CLI can also emit the current graph contract as versioned JSON. The export includes summary counts, filters, parse/read diagnostics, and the graph itself. The graph contains source-file, Django app, model, URL, view, serializer, form, service, middleware, context processor, signal, signal handler, and task nodes, plus containment, call, inheritance, direct ORM relationship, URL routing, serialization, query, global hook intercept, and trigger edges:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --format json --output graph.json
```

Use DOT output with Graphviz for visual rendering:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --format dot --output graph.dot
dot -Tsvg graph.dot > graph.svg
```

Large graphs can be narrowed with repeatable or comma-separated filters:

```sh
cargo run -p spelunking-cli -- /path/to/django-project \
  --format dot \
  --node-type url,view,model \
  --edge-type routes_to,queries \
  --path-prefix shop \
  --drop-isolated \
  --output shop-flow.dot
```

Current Django analysis includes:

- model discovery through direct and inherited Django model bases
- model metadata for `Meta` options, including `abstract` and `proxy`
- model manager discovery from `models.Manager`, custom manager classes, and `as_manager()`
- direct ORM relationships through `ForeignKey`, `OneToOneField`, and `ManyToManyField`
- field-level relationship attributes, including `through=...` and `related_name`
- reverse ORM relationship edges
- generic relation nodes for `GenericForeignKey`
- URL route discovery from `path(...)`, `re_path(...)`, and legacy `url(...)`
- function views and class-based views through `.as_view()`
- basic `include(...)` expansion
- basic DRF router registrations included through `router.urls`
- runtime context from `INSTALLED_APPS`, `ROOT_URLCONF`, `MIDDLEWARE`, and template context processors
- app config labels through `django.apps.AppConfig`
- middleware-to-URL intercept edges for configured request wrappers
- context-processor-to-URL intercept edges from `TEMPLATES[*]["OPTIONS"]["context_processors"]`
- signal receivers through `@receiver(...)` and `signal.connect(...)`
- Celery task discovery through `@shared_task` and `@app.task`
- trigger edges from models to signals, signals to handlers, and handlers to tasks
- service/helper function calls from views, handlers, tasks, and other services
- DRF `Serializer` / `ModelSerializer` discovery
- Django `Form` / `ModelForm` discovery
- `Meta.model` bindings from serializers/forms to models
- view references to serializers, forms, and direct ORM model usage

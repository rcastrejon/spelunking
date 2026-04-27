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

Generate operational guidance for the subject to highlight risk signals, open questions, coupling across apps/layers, related tests, and a short recommended reading path:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --inspect-guidance reservations.Reservation.status
```

Guidance is generated from the subject-focused behavioral slice produced by `--inspect-subject` and `--inspect-behavior`; it is not yet a literal `GraphExport` subgraph filtered to the subject. The risk and coupling signals are intentionally heuristic, so the output includes an analysis-basis section with data sources, slice counts, and caveats.

Extract candidate domain facts from one or more evidence packs for the subject. Domain facts translate technical evidence into proposed, reviewable domain knowledge with evidence, confidence, origin, basis, and review status:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --inspect-domain-facts reservations.Reservation.status
```

Pass multiple subjects as repeated values or comma-separated values to produce one merged fact set:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --inspect-domain-facts reservations.Reservation.status,payments.Payment.status
```

Already generated evidence-pack JSON files can be used without re-running project analysis:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --inspect-domain-facts-from-pack .domain-atlas/evidence-packs/reservations-reservation-status.json
```

Domain facts use schema version `1`. Each JSONL line includes `id`, `pack_id`, `statement`, `type`, `subject`, `technical_subject`, `primary_concept`, `field_concept`, `evidence`, `confidence`, `origin`, `basis`, `status`, `requires_human_review`, and `rationale`. Valid `type` values are `domain_concept_candidate`, `lifecycle_candidate`, `business_rule_candidate`, `flow_step`, `concept_relationship`, `boundary_risk`, `side_effect`, `open_question`, `pending_decision`, and `glossary_term_candidate`. Increment 1 emits `origin` values `programmatic` or `heuristic`, `basis` values `observed` or `inferred`, `status` value `proposed`, and `requires_human_review=true`; later review work may introduce `llm`, `human`, `confirmed`, `rejected`, and `stale`.

Generate consumable artifacts for humans and agents. By default these are written under `.domain-atlas` in the inspected project:

```sh
cargo run -p spelunking-cli -- /path/to/django-project --generate-artifacts reservations.Reservation.status
```

This writes:

- `.domain-atlas/evidence-packs/reservations-reservation-status.json`: compact JSON evidence pack for agents/LLMs
- `.domain-atlas/facts/domain-facts.jsonl`: candidate domain facts extracted from the evidence pack
- `.domain-atlas/reports/reservations-reservation-status-lifecycle.md`: short human lifecycle report
- `.domain-atlas/evaluation/reservations-reservation-status-evaluation.md`: scorecard for comparing manual exploration, a generic agent, and an agent using the evidence pack

Generate a single artifact with `--generate-evidence-pack`, `--generate-domain-facts`, `--generate-domain-facts-from-pack`, `--generate-report`, or `--generate-evaluation`. Use `--artifact-dir` to write somewhere else.

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

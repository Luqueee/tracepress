# Tracepress — MVP Implementation Plan

Quiero construir **Tracepress**, una capa local de optimización de contexto para agentes de IA.

El objetivo de Tracepress NO es simplemente reducir stdout o contar `bytes / 4`.

El objetivo principal es:

> **Minimizar el coste/tokens totales de una sesión de agente manteniendo la calidad, y proporcionar trazabilidad causal completa de cada transformación realizada sobre el contexto.**

Tracepress debe permitir saber:

* qué contenido iba a recibir originalmente el modelo;
* qué contenido recibió finalmente;
* qué transformación se aplicó;
* qué se eliminó;
* por qué se eliminó;
* cuánto se ahorró;
* cuánto costó realmente la sesión;
* si el agente tuvo que recuperar información;
* si repitió tools;
* si la compresión provocó errores/reintentos;
* qué policy produjo cada decisión;
* cuál fue el outcome posterior.

La arquitectura debe estar preparada desde el principio para generar datasets que permitan posteriormente entrenar/optimizar automáticamente la política de compresión.

---

# 1. Principios arquitectónicos

## 1.1 Tracepress NO debe interceptar principalmente stdout

No queremos repetir la arquitectura de herramientas que reemplazan:

```bash
grep ...
```

por:

```bash
tracepress grep ...
```

porque modificar stdout puede romper:

```bash
find ... | xargs ...
grep ... | wc -l
git ... > file
command | tee ...
```

Tracepress debe intervenir preferentemente **cuando el agente construye la request que va a mandar al LLM**.

Flujo:

```text
Tool executes normally
        │
        ▼
Agent receives tool result
        │
        ▼
Agent builds next LLM request
        │
        ▼
Tracepress Proxy
        │
        ├── observes original request
        ├── detects tool result blocks
        ├── optimizes eligible content
        ├── stores original if lossy
        └── forwards optimized request
        │
        ▼
LLM Provider
```

Los adapters específicos de agentes serán principalmente para:

* identificar sesiones;
* correlacionar tools;
* capturar metadata adicional;
* integración de recovery.

No deben ser el punto principal de compresión salvo que no exista otra alternativa.

---

# 2. Arquitectura principal

Implementar:

```text
                         Agent
                           │
                           ▼
                 Per-session ingress
                           │
                           ▼
                   ┌───────────────┐
                   │ tracepressd   │
                   └───────┬───────┘
                           │
        ┌──────────────────┼───────────────────┐
        │                  │                   │
        ▼                  ▼                   ▼
 Provider Adapter    Context Optimizer     Observability
        │                  │                   │
        │            Content Router             │
        │                  │                   │
        │          Compression Policy           │
        │                  │                   │
        │              Compressors              │
        │                  │                   │
        │           Recovery Storage            │
        │                  │                   │
        └──────────────────┼───────────────────┘
                           │
                           ▼
                     LLM Provider

                 ┌────────────────────┐
                 │ Storage            │
                 │                    │
                 │ SQLite             │
                 │ Hybrid Blob Store  │
                 └────────────────────┘

                           │
                    optional export
                           ▼

               OpenTelemetry / Parquet
                           │
                           ▼
                        DuckDB
```

---

# 3. Lenguaje y stack

Core:

```text
Rust
```

Preferencias:

```text
async runtime        Tokio
HTTP/proxy           Axum + Hyper
serialization        serde
SQLite               rusqlite
compression          zstd
hashing              SHA-256
tracing              tracing
OTel                  opentelemetry
IDs                   UUIDv7
```

Evitar dependencias grandes salvo que aporten una ventaja clara.

No introducir ML en el MVP.

---

# 4. `tracepressd`

Crear un daemon local.

Responsabilidades:

```text
- proxy HTTP
- session registry
- provider adapters
- context optimizer
- policy engine
- recovery API
- SQLite writer
- BlobStore
- GC
- telemetry exporter
```

NO permitir que múltiples hooks/procesos escriban directamente sobre SQLite.

Todos deben hablar con `tracepressd`.

Motivo:

SQLite soporta múltiples readers pero un único writer efectivo.

Arquitectura:

```text
Agent 1 ─┐
Agent 2 ─┤
CLI     ─┼── IPC ── tracepressd ── SQLite
Adapter ─┤
Proxy   ─┘
```

---

# 5. IPC

Crear abstracción:

```rust
trait IpcTransport
```

Implementaciones previstas:

```text
Linux/macOS → Unix Domain Socket
Windows     → Named Pipe
fallback    → localhost TCP autenticado
```

No acoplar el protocolo interno a Unix sockets.

---

# 6. Session isolation

Cada:

```bash
tracepress run <agent>
```

debe crear:

```text
session_id = UUIDv7
```

Preferiblemente asignar un ingress/proxy específico a esa sesión.

Ejemplo:

```text
session s_123

127.0.0.1:43191
       │
       ▼
tracepressd
```

De esta manera no dependemos exclusivamente de headers personalizados para saber a qué sesión pertenece una request.

---

# 7. Modelo de datos: NO usar Turn como primitive principal

Las sesiones de agentes no son secuencias simples.

Pueden existir:

```text
LLM
 ├── Tool A
 ├── Tool B
 └── Tool C
       ↓
      LLM

subagents
parallel tools
recoveries
provider tools
```

Modelar como DAG causal.

Primitive:

```rust
Operation
```

Tipos:

```rust
enum OperationKind {
    Agent,
    LlmInference,
    ToolExecution,
    Compression,
    Recovery,
    Evaluation,
    ProviderTool,
}
```

Y:

```rust
CausalEdge {
    parent_operation_id,
    child_operation_id,
    relationship,
}
```

Los turns pueden derivarse posteriormente para UI.

---

# 8. IDs

No usar autoincrement IDs como identificador principal externo.

Usar UUIDv7 para:

```text
session_id
operation_id
request_id
attempt_id
tool_call_id
compression_decision_id
recovery_id
evaluation_id
policy_assignment_id
```

SQLite puede seguir teniendo rowid internamente.

Para contenido:

```text
SHA256(raw_bytes)
```

---

# 9. Modelo de contenido

Nunca asumir que todo es UTF-8/String.

Base:

```rust
struct ContentObject {
    content_id: ContentId,
    raw_bytes: ...
}
```

Representación lógica:

```rust
enum ContentKind {
    Text,
    Json,
    Ndjson,
    Log,
    SearchResults,
    TestResults,
    SourceCode,
    Diff,
    Image,
    Document,
    Binary,
    Unknown,
}
```

Pipeline:

```text
raw bytes
    │
    ▼
safe decoder
    │
    ▼
normalized representation
    │
    ▼
content detection
```

El raw original nunca debe modificarse.

---

# 10. Distinguir cuatro niveles de contenido

Registrar por separado:

```text
process_output

agent_tool_result

tracepress_result

provider_input
```

Esto es crítico.

La baseline de ahorro debe ser principalmente:

```text
agent_tool_result
```

vs:

```text
tracepress_result
```

NO:

```text
process_output
```

porque el agente/harness puede haber truncado o modificado previamente el stdout.

---

# 11. Content bindings congelados

Una vez el modelo ha visto una representación comprimida de un resultado:

```text
raw @abc
   ↓
compressed @xyz
```

esa asociación debe quedar congelada dentro de esa sesión.

Crear:

```rust
ContextBinding {
    session_id,
    logical_content_id,
    raw_content_id,
    rendered_content_id,
    compressor_version,
    policy_version,
    frozen: true,
}
```

En turnos posteriores:

```text
@abc → siempre @xyz
```

Nunca recomprimir contextos históricos con una policy distinta.

Motivo:

* consistencia semántica;
* reproducibilidad;
* prompt caching.

---

# 12. Storage

Usar:

```text
SQLite
+
Hybrid BlobStore
```

SQLite:

```text
metadata
relations
small blobs
```

External CAS:

```text
large blobs
```

No imponer todavía un threshold definitivo.

Crear configurable:

```toml
[storage]
inline_blob_max_bytes = ...
```

Benchmarkear posteriormente.

---

# 13. SQLite config

Por defecto:

```sql
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
PRAGMA busy_timeout=5000;
```

Durability configurable:

```toml
[storage]
durability = "balanced"
```

Mapeo aproximado:

```text
balanced → synchronous=NORMAL
strict   → synchronous=FULL
```

---

# 14. Single writer

Toda persistencia operacional debe pasar por un writer controlado por `tracepressd`.

Separar:

## Critical state

Debe persistirse antes de enviar una transformación lossy:

```text
raw content
content binding
compression decision
recovery mapping
```

## Non-critical telemetry

Puede escribirse asynchronous/batched:

```text
debug events
histograms
OTel
analytics metadata
```

---

# 15. Regla fundamental de lossy compression

Nunca enviar una compresión lossy si el raw original no está recuperable.

Flujo obligatorio:

```text
compress
   │
   ▼
store raw/recovery
   │
 success?
  /      \
 no       yes
 │         │
 ▼         ▼
RAW     COMPRESSED
```

Si falla:

```text
disk
DB
CAS
serialization
```

hacer fail-open:

```text
send raw
```

---

# 16. Hybrid BlobStore

Interface:

```rust
trait BlobStore {
    put(...)
    get(...)
    exists(...)
    delete(...)
}
```

Implementaciones:

```text
InlineSQLiteBlobStore
FilesystemCasBlobStore
HybridBlobStore
```

Para blobs externos:

```text
1. write temp
2. SHA-256 incremental
3. zstd incremental
4. fsync según durability
5. atomic rename al path final
6. DB transaction registra referencia
```

Es preferible dejar un orphan blob que una DB apuntando a contenido inexistente.

Crear GC para orphans.

---

# 17. Blob pinning y GC

Un blob no puede borrarse si:

```text
active session references it
dataset references it
recovery can still request it
retention period active
```

Agregar concepto:

```text
pin_count
```

o referencias equivalentes.

GC debe ejecutarse desde el daemon/single writer.

---

# 18. Recovery API

Implementar:

```bash
tracepress get <content-id>
```

Soportar al menos:

```bash
tracepress get @id

tracepress get @id --lines 100:200

tracepress get @id --grep "timeout"

tracepress get @id --head 100

tracepress get @id --tail 100
```

Más adelante:

```text
--json-path
--symbol
--item
--file
```

También diseñar una tool estructurada:

```text
tracepress_retrieve
```

con schema pequeño.

---

# 19. Recovery no debe recomprimirse normalmente

Marcar el resultado:

```text
origin = tracepress_recovery
```

Por defecto:

```text
compression = bypass
```

o usar una policy mucho más conservadora.

Evitar:

```text
recover
→ compressor lo corta de nuevo
→ nuevo recover
```

---

# 20. Fidelity classes

Toda transformación debe declarar:

```rust
enum FidelityClass {
    Exact,
    Semantic,
    Lossy,
}
```

Ejemplos:

```text
EXACT
byte-equivalent reversible transformation

SEMANTIC
mismos datos pero representación distinta

LOSSY
información eliminada
```

Y separadamente:

```text
recoverable: bool
```

Un resultado común será:

```text
fidelity = LOSSY
recoverable = true
```

---

# 21. CompressionDecision

Crear como primitive central:

```rust
struct CompressionDecision {
    id: DecisionId,

    session_id: SessionId,
    operation_id: OperationId,

    input_content_id: ContentId,
    output_content_id: ContentId,

    content_kind: ContentKind,

    compressor: String,
    compressor_version: String,

    policy_version: String,

    fidelity: FidelityClass,
    recoverable: bool,

    raw_bytes: u64,
    output_bytes: u64,

    estimated_raw_tokens: Option<u64>,
    estimated_output_tokens: Option<u64>,

    target_tokens: Option<u64>,

    latency_us: u64,

    features: CompressionFeatures,
}
```

---

# 22. CompressionFeatures

Registrar desde el principio:

```rust
struct CompressionFeatures {
    duplication_ratio: Option<f32>,
    error_density: Option<f32>,
    entropy: Option<f32>,
    item_count: Option<u64>,
    line_count: Option<u64>,
    context_fill_ratio: Option<f32>,
    nesting_depth: Option<u32>,
}
```

Añadir schema/version:

```text
feature_schema_version
```

Esto será importante para training.

---

# 23. Provenance

Necesitamos saber:

```text
qué se eliminó
por qué
```

Pero NO crear una fila SQLite por línea.

Guardar resumen en SQLite:

```text
duplicate_lines_removed
noise_lines_removed
items_removed
errors_preserved
```

Y provenance detallado opcional como blob comprimido:

```text
decision
  │
  └── provenance_content_id
```

El provenance debe permitir mapear:

```text
raw byte ranges
→ action
→ reason
→ score
```

---

# 24. Content Router

Crear:

```rust
trait ContentDetector
```

Y:

```rust
ContentRouter
```

No routear únicamente por comando.

El comando/tool es una señal adicional.

Ejemplo:

```text
kubectl
aws
gh
docker
custom MCP
```

pueden producir JSON y compartir compressor.

---

# 25. Compressors iniciales

NO empezar con ML.

Primera generación:

## Plain text normalization

Solo seguro:

```text
ANSI removal
progress bars
redundant whitespace
exact duplicate collapse
```

## JSON

Primero lossless/semantic:

```text
homogeneous array
→ columns + rows
```

## Logs

```text
fingerprint
dedup
counts
preserve high-value errors
```

## Search results

```text
group by file
deduplicate
caps
context reduction
```

## Test output

```text
summary
failures
relevant stack
passing count
```

Code/semantic diff pueden ir después.

---

# 26. Hard budgets

Toda compresión debe tener un:

```rust
TokenBudget
```

Ejemplo:

```rust
struct TokenBudget {
    preferred: u64,
    hard_max: u64,
}
```

No permitir reglas infinitas tipo:

```text
preserve ALL errors
```

Un input adversarial puede contener millones de errores.

Errores deben tener prioridad alta, pero seguir dentro de un hard budget.

---

# 27. Protección contra inputs extremos

Imponer límites desde v0.1:

```text
max raw bytes
max decompressed bytes
max JSON nesting
max JSON items
max line length
max processing time
max CPU budget
```

Soportar:

```text
500MB logs
1GB JSON
invalid UTF-8
binary data
NUL bytes
decompression bombs
```

El sistema debe:

```text
stream
bypass
truncate safely
```

pero nunca OOM.

---

# 28. Streaming

Provider adapters deben soportar:

```text
SSE
chunked responses
client cancellation
disconnect
partial responses
```

Inference lifecycle:

```rust
enum InferenceStatus {
    Started,
    Streaming,
    Completed,
    Incomplete,
    Cancelled,
    Errored,
    Disconnected,
}
```

Usage:

```rust
enum UsageStatus {
    Final,
    Partial,
    Unavailable,
}
```

No inventar valores cuando el provider no los da.

---

# 29. No almacenar un evento por token

Capturar por defecto:

```text
TTFT
duration
chunk_count
bytes
usage
final assembled content
```

Chunks detallados:

```text
debug mode only
```

---

# 30. Provider usage

Crear dos conceptos:

```text
RawProviderUsage
NormalizedUsage
```

Guardar SIEMPRE el raw del provider.

Normalización:

```rust
struct NormalizedUsage {
    input_total: Option<u64>,
    input_uncached: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,

    output_total: Option<u64>,
    reasoning: Option<u64>,

    usage_status: UsageStatus,
}
```

Diferentes providers usan semánticas distintas.

No mapear campos únicamente por nombre.

---

# 31. Unknown != zero

Todo dato no disponible:

```text
NULL
```

Nunca:

```text
0
```

si realmente significa unknown.

Aplicar a:

```text
tokens
reasoning
cost
cache
latency components
quality
```

---

# 32. Provider attempts

Una request lógica puede tener:

```text
attempt 1 → 429
attempt 2 → 500
attempt 3 → success
```

Modelar:

```text
provider_request
  └── provider_attempt[]
```

El coste/latencia real debe incluir intentos si correspondiera.

No contar retries como requests independientes sin causalidad.

---

# 33. Cost model

Tokens != coste.

Crear:

```rust
UsageVector
```

con categorías extensibles.

Crear:

```rust
CostModel
```

versionado.

Guardar:

```text
pricing_snapshot
```

Costes en:

```text
integer microusd
```

o Decimal.

No usar float para dinero.

---

# 34. Schema canónico propio

NO usar OpenTelemetry GenAI como modelo interno.

Crear:

```text
Tracepress Canonical Schema
```

versionado:

```text
schema_version
```

Después:

```text
Canonical Schema
      │
      ▼
OTel Mapper
      │
      ▼
OTLP
```

Las convenciones de OTel pueden cambiar sin obligar a migrar el modelo interno.

---

# 35. IDs de OTel separados

Mantener:

```text
tracepress_session_id
tracepress_operation_id
```

separados de:

```text
otel_trace_id
otel_span_id
```

Guardar mapping.

---

# 36. Privacy

Por defecto:

```text
local raw content
external telemetry = metadata only
```

No exportar automáticamente:

```text
prompts
tool outputs
Authorization headers
API keys
env vars
absolute paths
user source code
```

Implementar redaction layer.

---

# 37. Sensitive hashes

Internamente:

```text
SHA256(raw)
```

Para IDs exportados:

```text
HMAC-SHA256(local_secret, content_id)
```

Evitar enviar hashes determinísticos del contenido raw a servidores externos.

---

# 38. Secrets

Añadir sanitización antes de persistir/exportar:

```text
Authorization
Bearer
API keys
AWS secrets
GitHub tokens
OpenAI keys
Anthropic keys
cookies
```

Los headers completos del provider NO deben almacenarse.

---

# 39. SQLite schema mínimo

Diseñar inicialmente:

```text
schema_metadata

sessions
operations
causal_edges

provider_requests
provider_attempts
provider_usage

content_objects
content_occurrences
content_bindings

compression_decisions
recoveries

policy_assignments
evaluations

events
```

---

# 40. Event log

Mantener append-only event log:

```text
events
```

Campos:

```text
seq
event_id
session_id
operation_id
timestamp
event_type
payload
schema_version
```

Pero NO usar JSON-events como única representación.

Las tablas relacionales siguen siendo necesarias para analytics/query.

---

# 41. Policy Engine

Separar completamente:

```text
Compressor
```

de:

```text
Policy
```

El compressor responde:

```text
"puedo transformar esto"
```

La policy responde:

```text
"usa este compressor con estos parámetros"
```

Interfaz aproximada:

```rust
trait Policy {
    fn decide(
        &self,
        context: &DecisionContext
    ) -> PolicyAction;
}
```

---

# 42. PolicyAction

Registrar:

```rust
struct PolicyAction {
    compressor,
    parameters,
    target_budget,
}
```

Desde el principio guardar:

```text
policy_version
feature_schema_version
```

---

# 43. Preparación para contextual bandits

Aunque v0.1 no use ML, guardar campos suficientes para aprender después:

```text
chosen_action
candidate_actions
action_probability
random_seed
policy_version
feature_vector
```

Sin esto, el off-policy evaluation futuro será mucho más difícil.

---

# 44. Outcome model

No optimizar contra una única scalar metric.

Guardar:

```rust
struct Outcome {
    provider_cost: Option<...>,

    input_tokens: Option<u64>,
    cache_tokens: Option<u64>,
    output_tokens: Option<u64>,

    recoveries: u32,
    reruns: u32,

    latency_ms: u64,

    task_success: Option<bool>,
    user_correction: Option<bool>,
    quality_score: Option<f32>,
}
```

Después distintas policies podrán definir diferentes objectives.

---

# 45. Recovery NO equivale a fracaso completo

Usar:

```text
recovery = evidence de información insuficiente
```

pero:

```text
no recovery != evidence definitiva de éxito
```

Una mala compresión puede producir una respuesta incorrecta sin recovery.

Combinar:

```text
tests
build outcome
task completion
user correction
reruns
recoveries
evaluators
cost
```

---

# 46. Shadow mode

Implementar antes de activar compresión en producción.

```bash
tracepress shadow on
```

Flujo:

```text
original request ──────────► provider

        │
        └────► optimizer
                │
                ▼
             candidate
                │
           stored only
```

Registrar:

```text
would_have_saved
candidate_policy
candidate_output
```

sin modificar la experiencia real.

---

# 47. A/A testing

Antes de A/B:

```text
50% baseline A
50% baseline B
```

ambos idénticos.

Comprobar:

```text
cost
latency
retries
success
```

No debe haber diferencias significativas.

Si las hay, hay bugs de instrumentación/assignment.

---

# 48. A/B testing

Assignment inicial:

```text
PER SESSION
```

No por tool.

Ejemplo:

```text
session
  │
 random
  │
 ├── control
 └── policy-v1
```

Esto permite estimar diferencias causales de session cost/outcome.

---

# 49. Replay

Replay sirve para:

```text
compression ratio
candidate evaluation
semantic preservation
token estimation
policy simulation
```

NO afirmar:

```text
"esto habría ahorrado exactamente X% de session cost"
```

porque cambiar el contexto puede cambiar la trayectoria del agente.

Diferenciar claramente:

```text
observed
estimated
counterfactual
```

en todas las métricas.

---

# 50. Parquet / DuckDB

SQLite es source of truth local.

Parquet:

```text
immutable training snapshot
```

DuckDB:

```text
offline analytics only
```

NO usar DuckDB en hot path.

Comando futuro:

```bash
tracepress dataset export experiment-42
```

Genera:

```text
manifest.json
sessions.parquet
operations.parquet
decisions.parquet
outcomes.parquet
policy_assignments.parquet
```

---

# 51. Network filesystem detection

`tracepress doctor` debe detectar si SQLite está sobre:

```text
NFS
network mount
unsupported filesystem
```

y avisar/obligar a usar storage local para WAL.

---

# 52. Fail-open

Regla general:

Si Tracepress falla:

```text
compression
DB auxiliary telemetry
OTel
analytics
policy
```

el agente debe poder continuar.

Especialmente:

```text
optimizer error
→ forward original request
```

Nunca bloquear una sesión por una feature de optimización.

---

# 53. Edge cases obligatorios

Crear tests para:

```text
kill -9 durante compression
disk full
SQLite busy
SQLite corruption detection
CAS write failure
GC + recovery race
NFS storage
stream cancelled
SSE malformed
provider 429
provider 500
provider reconnect
SDK retry
parallel tool calls
parallel subagents
history resent
same raw content in different contexts
policy update mid-session
recovery being recompressed
1 GB output
infinite output
invalid UTF-8
binary content
NUL bytes
deep JSON
millions of errors
decompression bomb
malicious ANSI
secret in command
secret in output
OTel backend offline
telemetry queue full
model change mid-session
provider server tools
provider-native compaction
migration interrupted
power loss
```

---

# 54. Property tests

Ejemplos:

```text
EXACT compressor:
decompress(compress(x)) == x
```

```text
LOSSY decision:
recovery_id != NULL
```

```text
LOSSY output emitted:
recovery blob must already exist
```

```text
frozen binding:
same logical content in same session
always produces same representation
```

---

# 55. Fuzzing

Fuzz:

```text
provider HTTP parsers
SSE parsers
JSON detector
JSON compressor
log parser
recovery selector
decoders
content router
```

---

# 56. Implementación por fases

## PHASE 0 — Repository architecture

Crear workspace Rust:

```text
crates/
├── tracepress-core
├── tracepress-storage
├── tracepress-proxy
├── tracepress-provider
├── tracepress-compression
├── tracepress-policy
├── tracepress-telemetry
├── tracepress-ipc
├── tracepress-daemon
└── tracepress-cli
```

No sobrefragmentar innecesariamente, pero mantener boundaries claros.

---

## PHASE 1 — Transparent infrastructure

Implementar:

```text
tracepressd
IPC
SQLite migrations
Hybrid BlobStore
session lifecycle
operation DAG
transparent proxy
```

NO compression.

Acceptance:

```text
request in == request out
response in == response out
```

salvo transport metadata.

---

## PHASE 2 — Provider observability

Elegir UN provider/protocolo primero.

Implementar completamente:

```text
normal requests
streaming
errors
retries
cancellation
usage
cache usage
reasoning usage
cost normalization
```

Guardar raw usage.

---

## PHASE 3 — Shadow mode

Implementar:

```text
ContentRouter
CompressionDecision
policy baseline
candidate compression
```

pero NO modificar requests.

Generar datasets reales.

---

## PHASE 4 — Safe compression

Activar únicamente:

```text
EXACT
SEMANTIC
```

Transformaciones demostrablemente seguras.

---

## PHASE 5 — Recovery

Implementar:

```text
LOSSY
CAS guarantee
tracepress_retrieve
targeted retrieval
```

---

## PHASE 6 — Experimental infrastructure

Implementar:

```text
A/A
session-level A/B
control holdout
policy assignment logging
```

---

## PHASE 7 — Analytics

Implementar:

```text
tracepress stats
tracepress inspect
tracepress trace
tracepress dataset export
Parquet
DuckDB optional
```

---

## PHASE 8 — Adaptive policy

NO implementar hasta tener suficientes datos.

Primero:

```text
rules
```

Después experimentar offline con:

```text
gradient boosting
LinUCB
Thompson Sampling
contextual bandits
```

---

# 57. CLI inicial

```bash
tracepress init

tracepress daemon start
tracepress daemon stop
tracepress daemon status

tracepress run <agent>

tracepress proxy

tracepress doctor

tracepress stats

tracepress trace <session-id>

tracepress inspect <decision-id>

tracepress get <content-id>

tracepress shadow on
tracepress shadow off

tracepress dataset export
```

---

# 58. Métricas fundamentales

Nunca presentar únicamente:

```text
compression ratio
```

Mostrar:

```text
raw_tool_tokens
agent_tool_result_tokens
forwarded_tokens

gross_tokens_saved

recovered_tokens
recovery_rate
tool_rerun_rate

provider_input_tokens
provider_cache_read_tokens
provider_cache_write_tokens
provider_output_tokens
provider_reasoning_tokens

provider_cost

compression_latency

baseline_session_cost
optimized_session_cost

net_session_cost_saved
```

Etiquetar claramente:

```text
observed
estimated
counterfactual
```

---

# 59. Métrica principal futura

La métrica comercial/técnica principal debe ser:

```text
NET SESSION COST SAVING
```

y secundariamente:

```text
NET SESSION TOKEN SAVING
```

No:

```text
stdout compression %
```

---

# 60. Requisito final

El activo principal de Tracepress debe ser un dataset causal y auditable de:

```text
context
+
policy features
+
decision/action
+
compression provenance
+
provider usage
+
downstream agent behavior
+
outcome
```

La infraestructura debe diseñarse alrededor de esto.

No priorizar todavía:

```text
ML compressor
embeddings
vector database
cloud dashboard
multi-agent memory
RL
SaaS
```

Antes debemos conseguir que las mediciones sean correctas.

---

# Definition of Done del MVP

Considerar el MVP técnicamente válido cuando:

1. Podemos reconstruir una sesión completa mediante su DAG de operaciones.

2. Podemos relacionar cada provider request con los tool results que contiene.

3. Sabemos exactamente qué representación se mandó al provider.

4. Cada compresión genera una `CompressionDecision`.

5. Cada transformación lossy tiene recovery garantizado.

6. Una representación histórica está frozen dentro de su sesión.

7. Tenemos provider usage real cuando esté disponible.

8. `unknown` nunca se representa como `0`.

9. Podemos distinguir gross savings de net savings.

10. Shadow mode funciona sin alterar las requests.

11. A/A testing demuestra que la instrumentación no introduce sesgos importantes.

12. Podemos exportar un dataset reproducible para análisis offline.

13. Un fallo del optimizer nunca impide continuar al agente.

14. Tracepress mantiene memoria/latencia acotadas ante outputs gigantes.

15. Los contenidos sensibles permanecen locales por defecto.

16. El esquema interno está versionado y es independiente de OpenTelemetry.

17. Las migraciones son testeadas.

18. Tenemos tests de crash consistency y recovery.

19. Tenemos fuzzing sobre parsers de inputs no confiables.

20. Toda optimización futura puede evaluarse contra un control real.

---

# Instrucción para la implementación

Antes de escribir grandes cantidades de código:

1. Revisa críticamente este diseño.
2. Señala cualquier contradicción o edge case adicional.
3. Define los invariantes arquitectónicos.
4. Diseña el schema SQLite v1.
5. Diseña las traits principales.
6. Diseña el flujo exacto de una request.
7. Diseña primero los tests de crash consistency y fail-open.
8. Solo después empieza a implementar Phase 0 y Phase 1.

No introduzcas ML ni features fuera del MVP salvo que sean necesarias para evitar una mala decisión arquitectónica irreversible.

Prioridades:

```text
correctness
> recoverability
> observability
> reproducibility
> performance
> compression ratio
```

La optimización de tokens debe construirse sobre datos fiables, no al revés.

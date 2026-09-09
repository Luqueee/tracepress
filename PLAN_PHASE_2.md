# Tracepress — Phase 2: Provider Observability

## Context

Tracepress Phase 0/1 está finalizada.

Baseline obligatorio:

```text
commit: 7824936
branch: main
```

NO reabrir ni ampliar retrospectivamente Phase 0/1.

Preservar todos sus invariantes:

```text
correctness
> recoverability
> observability
> reproducibility
> performance
> compression ratio
```

En particular:

* single-writer SQLite;
* raw bytes preservados;
* BlobStore híbrido;
* CAS crash-safe;
* fail-open;
* DAG causal;
* IPC autenticado;
* inputs acotados;
* `unknown = NULL`;
* secretos fuera de logs/storage no autorizado;
* migraciones incrementales;
* forwarding transparente.

---

# 1. Objetivo de Phase 2

Implementar **observabilidad semántica real del proveedor**.

Tracepress debe pasar de saber:

```text
HTTP request:
  14,821 bytes

HTTP response:
  8,281 bytes
  status: 200
```

a saber:

```text
Provider:
  OpenAI

Protocol:
  Responses API v1

Request:
  model: gpt-5.6-sol
  stream: true
  input structure: ...
  tools: 12
  reasoning effort: medium

Response:
  id: resp_...
  status: completed
  model: ...
  finish state: completed

Usage:
  input_total: 18,291
  input_cached: 12,800
  input_uncached: 5,491

  output_total: 2,183
  reasoning: 1,422

  total: 20,474

Usage status:
  final
```

Sin modificar todavía el contexto enviado al modelo.

---

# 2. NO implementar todavía

Fuera de scope:

```text
❌ context compression
❌ lossy compression
❌ semantic compression
❌ recovery tool
❌ ContentRouter activo
❌ policy optimization
❌ ML
❌ embeddings
❌ OpenTelemetry
❌ Parquet
❌ DuckDB
❌ A/A
❌ A/B
❌ contextual bandits
❌ cost optimization
❌ dashboard
❌ múltiples providers
```

Los crates:

```text
tracepress-compression
tracepress-policy
tracepress-telemetry
```

pueden adquirir interfaces necesarias para integración futura, pero no funcionalidad de fases posteriores.

---

# 3. Provider objetivo

Phase 2 debe implementar exactamente:

```text
Provider: OpenAI
Protocol: Responses API v1
Endpoint: POST /v1/responses
```

El soporte existente:

```text
POST /v1/chat/completions
```

debe continuar funcionando byte-exact y pasar todos sus tests actuales.

Pero no convertir Chat Completions en un segundo parser semántico durante esta fase.

Objetivo:

```text
/v1/chat/completions
    → transparent legacy path

/v1/responses
    → transparent forwarding
      +
      semantic observation
```

Esto evita construir dos protocolos simultáneamente.

---

# 4. Principio fundamental

Provider observability debe ser un **side channel**.

Arquitectura:

```text
                   request bytes
Agent ──────────────────────────────────► OpenAI
                 │
                 │ copy/tap
                 ▼
          Semantic Observer
                 │
                 ▼
              SQLite
```

Nunca:

```text
Agent
  │
  ▼
Parse entire request
  │
  ▼
serialize again
  │
  ▼
OpenAI
```

NO queremos parse → reserialize en el hot forwarding path.

El proveedor debe continuar recibiendo los bytes originales.

---

# 5. Invariante de transparencia

Para Phase 2:

```text
forwarded_request_bytes == original_request_bytes
```

y cuando aplique:

```text
forwarded_response_bytes == upstream_response_bytes
```

La observabilidad puede:

```text
succeed
fail
be partial
be unsupported
drop events
```

sin modificar los bytes que recibe el agente.

---

# 6. ProviderProtocol abstraction

Crear una abstracción versionada.

Por ejemplo:

```rust
pub trait ProviderObserver {
    fn provider(&self) -> ProviderKind;

    fn protocol(&self) -> ProviderProtocol;

    fn observe_request(
        &self,
        input: ObservationInput<'_>,
    ) -> ObservationResult<RequestObservation>;

    fn observe_response(
        &self,
        input: ObservationInput<'_>,
    ) -> ObservationResult<ResponseObservation>;
}
```

Tipos aproximados:

```rust
enum ProviderKind {
    OpenAi,
}
```

```rust
enum ProviderProtocol {
    OpenAiResponsesV1,
}
```

No usar:

```text
if provider == "openai"
```

disperso por el código.

El conocimiento específico debe vivir dentro del adapter.

---

# 7. Versionar parsers

Cada parser debe tener identidad explícita:

```text
provider = openai
protocol = responses-v1
parser_version = 1
```

Persistir:

```text
provider
protocol
parser_version
```

junto con la observación.

Esto permitirá en el futuro:

```text
parser v1
      ↓
bug discovered
      ↓
parser v2
      ↓
backfill historical records
```

sin perder reproducibilidad.

---

# 8. Observation status

Crear:

```rust
enum ObservationStatus {
    Complete,
    Partial,
    Unsupported,
    Malformed,
    ResourceLimit,
    ObserverBackpressure,
    Cancelled,
}
```

Nunca representar:

```text
parse failed
```

como:

```text
usage = 0
```

Debe conservarse la diferencia.

---

# 9. Request observation

Extraer de `/v1/responses` solo metadata útil y segura.

Ejemplo:

```rust
struct OpenAiResponsesRequestObservation {
    model: Option<String>,

    stream: Option<bool>,
    background: Option<bool>,
    store: Option<bool>,

    reasoning_effort: Option<String>,
    verbosity: Option<String>,
    truncation: Option<String>,

    has_previous_response_id: bool,

    input_item_count: Option<u64>,
    tool_count: Option<u64>,

    text_input_blocks: Option<u64>,
    image_input_blocks: Option<u64>,
    file_input_blocks: Option<u64>,

    parser_version: u32,
}
```

NO persistir indiscriminadamente:

```text
input text
instructions
metadata
tool arguments
prompt_cache_key
user identifiers
```

durante Phase 2.

---

# 10. Content capture policy

Provider observability y content capture son cosas distintas.

Crear explícitamente:

```rust
enum ContentCaptureMode {
    Off,
    MetadataOnly,
    LocalRaw,
}
```

Phase 2 default:

```text
MetadataOnly
```

No empezar a persistir prompts completos silenciosamente.

`LocalRaw` debe existir como capacidad experimental pero:

* nunca exportarse;
* usar CAS existente;
* respetar límites;
* registrar claramente que está activo;
* requerir configuración explícita.

No implementar todavía cloud content export.

---

# 11. No persistir secretos

Nunca guardar:

```text
Authorization
cookies
API keys
full request headers
full response headers
```

Mantener allowlist de metadata HTTP.

No blacklist.

Ejemplo:

```rust
AllowedResponseMetadata {
    content_type,
    content_length,
    status,
}
```

Si posteriormente se captura provider request ID, hacerlo como campo explícitamente permitido.

---

# 12. Response lifecycle

Responses API no debe modelarse como:

```text
request → success/error
```

Crear lifecycle.

Ejemplo:

```rust
enum ProviderResponseState {
    Queued,
    InProgress,
    Completed,
    Incomplete,
    Failed,
    Cancelled,
    Disconnected,
    Unknown,
}
```

Registrar:

```text
provider_response_id
model
created_at
completed_at
status
incomplete_reason
error_code
```

cuando estén disponibles.

---

# 13. Usage canonical model

Crear un modelo interno independiente de OpenAI.

```rust
struct NormalizedUsage {
    input_total: Option<u64>,

    input_cached: Option<u64>,
    input_uncached: Option<u64>,

    cache_write: Option<u64>,

    output_total: Option<u64>,
    output_reasoning: Option<u64>,

    total: Option<u64>,

    status: UsageStatus,

    normalizer_version: u32,
}
```

Y:

```rust
enum UsageStatus {
    Final,
    Partial,
    Unavailable,
}
```

---

# 14. Raw usage siempre preservado

Además de la normalización guardar:

```text
provider_usage_raw
```

pero SOLO el objeto `usage`, no la respuesta completa.

Por ejemplo:

```json
{
  "input_tokens": 18291,
  "input_tokens_details": {
    "cached_tokens": 12800
  },
  "output_tokens": 2183,
  "output_tokens_details": {
    "reasoning_tokens": 1422
  },
  "total_tokens": 20474
}
```

Guardar el raw permite:

```text
normalizer v1
      ↓
bug
      ↓
normalizer v2
      ↓
historical backfill
```

---

# 15. Normalización OpenAI Responses

Cuando los valores existan:

```text
input_total
    =
usage.input_tokens
```

```text
input_cached
    =
usage.input_tokens_details.cached_tokens
```

```text
input_uncached
    =
input_total - input_cached
```

solo si:

```text
input_total != NULL
AND
input_cached != NULL
AND
input_cached <= input_total
```

Si no:

```text
input_uncached = NULL
```

Nunca saturar silenciosamente a cero.

---

# 16. Output usage

Mapear:

```text
output_total
    =
usage.output_tokens
```

```text
output_reasoning
    =
usage.output_tokens_details.reasoning_tokens
```

```text
total
    =
usage.total_tokens
```

Todos opcionales.

---

# 17. Usage consistency checks

Validaciones:

```text
input_cached <= input_total

output_reasoning <= output_total

input_total + output_total == total
```

solo cuando todos los valores implicados existen.

Una inconsistencia del provider:

```text
NO debe romper forwarding
```

Debe generar:

```text
usage_normalization_anomaly
```

y conservar raw usage.

---

# 18. No calcular coste todavía

Phase 2 debe almacenar:

```text
model
usage
timestamp
provider
```

pero NO introducir todavía:

```text
model → current price
```

ni:

```text
cost_usd
```

Motivo:

los precios son mutables.

Una futura:

```text
CostModel(version)
```

podrá calcular costes históricos usando:

```text
provider
model
usage
timestamp
```

No contaminar provider observability con pricing.

---

# 19. Non-streaming responses

Implementar parser completo para respuestas JSON no-streaming.

Casos:

```text
completed
incomplete
failed
malformed JSON
oversized JSON
unknown fields
missing usage
missing model
missing status
```

Unknown fields deben ignorarse.

No exigir que el schema recibido coincida exactamente con el conocido.

---

# 20. Forward compatibility

Los structs de parsing no deben hacer que una nueva propiedad añadida por OpenAI rompa Tracepress.

Usar parsing tolerante:

```text
known fields → interpret
unknown fields → ignore
```

Pero conservar:

```text
schema anomaly
```

cuando un campo conocido tiene un tipo imposible.

---

# 21. Streaming architecture

Esta es una parte crítica de Phase 2.

NO acumular todo el stream antes de reenviarlo.

Arquitectura:

```text
                         ┌──────────────► client
                         │
upstream SSE ────────────┤
                         │
                         └──► observer tap
                                  │
                                  ▼
                              SSE parser
```

El forwarding es prioritario.

---

# 22. Observer tap no bloqueante

Usar canal acotado:

```text
upstream chunks
      │
      ├── forwarding
      │
      └── try_send(observer)
```

Si observer está saturado:

```text
forwarding continues
observation_status = ObserverBackpressure
```

Nunca:

```text
slow SQLite/parser
      ↓
slows model stream
```

---

# 23. Streaming parser incremental

No concatenar todo SSE en memoria.

Parser incremental:

```text
bytes
 ↓
SSE framing
 ↓
event
 ↓
provider semantic event
```

Debe soportar boundaries arbitrarios:

```text
"data:"
```

puede quedar dividido entre chunks TCP.

Tests explícitos:

```text
d | ata:
da | ta:
data | :
data: {js | on}
```

---

# 24. Estados SSE relevantes

Registrar al menos lifecycle suficiente para detectar:

```text
created
queued
in progress
completed
incomplete
failed/error
```

No persistir un row por token delta.

---

# 25. Streaming metrics

Registrar:

```text
started_at

first_upstream_byte_at
first_semantic_output_at

completed_at

chunk_count
byte_count

status
usage_status
```

Permite derivar:

```text
TTFB
TTFT
stream duration
```

sin almacenar todos los chunks.

---

# 26. Interrupted streams

Si:

```text
client disconnect
network disconnect
upstream error
cancel
```

antes del evento final:

```text
response_state = Disconnected/Cancelled/Incomplete
```

según la evidencia disponible.

Y:

```text
usage_status = Unavailable
```

si nunca llegó usage.

Nunca:

```text
usage = 0
```

---

# 27. Provider request vs provider attempt

Conservar la distinción:

```text
logical request
       │
       ├── attempt 1
       ├── attempt 2
       └── attempt 3
```

No asumir todavía que Tracepress realiza retries.

Pero el modelo de datos debe permitirlos.

Registrar por attempt:

```text
started_at
finished_at
HTTP status
transport error
response id
```

---

# 28. Correlación con DAG

Cada provider request debe estar asociado a:

```text
LlmInference Operation
```

Ejemplo:

```text
Agent
  │
  └─spawned→ LlmInference
                 │
                 └─provider_request
                       │
                       └─provider_attempt
```

No crear operaciones huérfanas.

---

# 29. SQLite migration

NO modificar:

```text
0001_initial.sql
```

Crear:

```text
0002_provider_observability.sql
```

Antes de diseñarla:

1. inspeccionar schema actual;
2. identificar qué columnas/tablas ya existen;
3. evitar duplicación;
4. preferir additive migration.

---

# 30. Campos necesarios

Ajustar al schema existente, pero conceptualmente necesitamos almacenar:

```text
provider
protocol
parser_version

request semantic status

response id
response state
model

raw usage JSON
normalized usage

normalizer version

observation status

streaming boolean

started timestamps
finished timestamps
TTFB
TTFT

parse/anomaly metadata
```

No crear tablas redundantes si `provider_requests`, `provider_attempts` y `provider_usage` ya ofrecen el boundary correcto.

---

# 31. Event log

Emitir eventos canónicos como:

```text
provider.request.observed
provider.response.started
provider.response.completed
provider.response.incomplete
provider.response.failed
provider.usage.observed
provider.usage.normalized
provider.observation.partial
```

No convertir `events` en el único source of truth.

Mantener tablas relacionales.

---

# 32. Backfill

Implementar una API interna de:

```text
RawProviderObservation
      ↓
ProviderParser
      ↓
SemanticObservation
```

que NO dependa del socket HTTP activo.

Esto permitirá ejecutar posteriormente:

```text
parser v2
```

sobre fixtures o contenidos capturados anteriormente.

No implementar todavía un gran CLI de replay.

Solo garantizar que el parser es reusable offline.

---

# 33. Real fixtures

Crear fixtures anonimizados de:

```text
Responses non-stream completed

Responses non-stream incomplete

Responses streaming completed

Responses streaming incomplete

stream cancellation

usage present

usage missing

cached input

reasoning output

tool calls

parallel tool calls

malformed SSE

unknown event

unknown JSON field

large response
```

No crear todos los tests únicamente con JSON inventado mínimo.

---

# 34. Golden fixtures

Cada fixture debe tener:

```text
raw_request
raw_response

expected_semantics.json
```

Ejemplo:

```text
fixtures/openai/responses/v1/
    completed_basic/
        request.json
        response.json
        expected.json
```

Para streaming:

```text
completed_stream/
    request.json
    response.sse
    expected.json
```

---

# 35. Fragmentation fixtures

El mismo `response.sse` debe probarse con:

```text
chunk size 1 byte
chunk size 2
chunk size 7
chunk size 64
random chunk boundaries
```

El resultado semántico debe ser idéntico.

Property:

```text
parse(fragment(x)) == parse(x)
```

para cualquier fragmentación válida.

---

# 36. Fuzzing

Añadir fuzz targets para:

```text
SSE framing
Responses JSON parser
usage parser
usage normalizer
stream state machine
```

Invariantes:

```text
never panic
bounded memory
bounded nesting
fail-open
```

---

# 37. Resource limits

Reutilizar los límites implementados en Phase 0/1.

Añadir específicos solo si faltan:

```text
max semantic request bytes
max semantic response bytes
max SSE event bytes
max usage object bytes
max event count
max JSON depth
max parser CPU time
```

No crear límites paralelos inconsistentes con `tracepress-core`.

---

# 38. Malicious provider payload

Tratar upstream como input no confiable.

Tests:

```text
huge JSON string
deep JSON
duplicate JSON keys
invalid UTF-8
NUL
huge SSE event
never-ending SSE event
millions of SSE events
malformed content-length
unexpected content-type
```

Forwarding debe continuar siempre que la capa HTTP pueda hacerlo.

Semantic observer puede abandonar.

---

# 39. Content-Type mismatch

Ejemplo:

```text
Content-Type: application/json
body: garbage
```

Forwarding:

```text
unchanged
```

Observation:

```text
Malformed
```

No devolver un error propio de Tracepress al cliente.

---

# 40. Privacy tests

Añadir tests que introduzcan canaries:

```text
sk-secret-TRACEPRESS_CANARY
Authorization: Bearer ...
cookie=...
password=...
```

Después inspeccionar:

```text
SQLite
logs
Debug output
IPC
events
```

y verificar que no aparecen.

---

# 41. Metadata fields potentially sensitive

No persistir directamente:

```text
prompt_cache_key
safety_identifier
user
metadata values
```

Si en el futuro interesan para correlación:

```text
HMAC(local_secret, value)
```

pero fuera de Phase 2 salvo necesidad justificada.

---

# 42. Shadow semantics

Phase 2 NO es todavía el `shadow mode` de compresión.

Pero conceptualmente provider parsing funciona en modo shadow:

```text
bytes forwarded
       │
       └──── semantic interpretation
```

Una interpretación incorrecta nunca afecta al provider request.

---

# 43. Compatibility tests

Todos los tests Phase 0/1 deben seguir pasando.

Especialmente:

```text
binary payload byte-exact
streaming/chunked
429
500
metadata sink failure
crash/restart
SQLite busy
CAS failures
```

No relajar tests existentes para conseguir Phase 2.

---

# 44. Unix / Windows decision

Para Phase 2 declarar oficialmente:

```text
Runtime target:
Linux + macOS

Windows:
not yet officially supported
```

NO implementar Windows IPC durante esta fase.

Sí mantener:

```text
portable abstractions
compileability where practical
```

Portabilidad completa debe tener un plan separado.

---

# 45. CI

Añadir CI real.

Gates obligatorios:

```bash
cargo fmt --all -- --check

cargo clippy \
  --workspace \
  --all-targets \
  --all-features \
  -- -D warnings

cargo test --workspace

cargo build --workspace
```

Añadir:

```text
Linux
macOS
```

Si es viable:

```text
Windows compile-only
```

sin prometer runtime support.

---

# 46. Fuzz CI

No ejecutar fuzzing infinito.

Crear:

```text
nightly/manual fuzz job
```

más smoke corto opcional en PR.

---

# 47. Reproducible build preparation

No hace falta publicar release todavía.

Pero dejar preparado:

```text
locked dependencies
documented Rust version
build metadata
release profile
artifact naming
```

No mezclar Phase 2 con Homebrew/package managers.

---

# 48. Observability metrics de Phase 2

Al terminar debemos poder consultar por sesión:

```text
requests
successful responses
failed responses
incomplete responses

streamed requests

input tokens
cached input tokens
uncached input tokens

output tokens
reasoning tokens

usage unavailable count

parser complete %
parser partial %
parser failed %

TTFB
TTFT
response duration
```

Todavía NO:

```text
tokens saved
net saving
compression ratio
```

porque aún no estamos optimizando nada.

---

# 49. CLI mínima adicional

Agregar solamente si aporta diagnóstico real:

```bash
tracepress trace <session-id>
```

o:

```bash
tracepress provider stats
```

Pero no construir dashboard.

Una salida suficiente:

```text
Session s_...

Provider      OpenAI
Protocol      responses-v1

Requests      18
Completed     17
Incomplete     1

Input         182,917
Cached        121,202
Uncached       61,715

Output         27,281
Reasoning      18,921

Usage final    17
Usage unknown   1

TTFT p50       ...
TTFT p95       ...
```

Si implementar esta CLI fuerza demasiado scope, posponerla y validar mediante tests/SQLite.

---

# 50. Phase 2 task breakdown

## T1 — Design freeze

Crear:

```text
docs/design/phase-2-provider-observability.md
```

Definir:

* provider abstraction;
* parser lifecycle;
* usage semantics;
* streaming model;
* privacy;
* limits;
* failure behavior;
* migration;
* non-goals.

No implementar antes de aprobar estos contratos.

---

## T2 — OpenAI Responses transparent endpoint

Añadir:

```text
POST /v1/responses
```

al proxy.

Primero exclusivamente forwarding.

Acceptance:

```text
request byte exact
response byte exact
stream byte exact
error byte exact
```

---

## T3 — Canonical provider domain

Implementar:

```text
ProviderKind
ProviderProtocol
ObservationStatus
ProviderResponseState
UsageStatus
RawProviderUsage
NormalizedUsage
```

Sin HTTP parsing todavía.

---

## T4 — SQLite migration 0002

Implementar persistencia necesaria.

No modificar 0001.

Tests:

```text
empty → v1 → v2
existing v1 → v2
interrupted migration
future schema rejection
rollback
NULL preservation
```

---

## T5 — Non-stream request parser

Implementar observation del request Responses v1.

Fail-open.

Resource bounded.

---

## T6 — Non-stream response parser

Implementar:

```text
completed
incomplete
failed
usage
```

más normalizer.

---

## T7 — Streaming observer

Implementar:

```text
bounded tap
incremental SSE parser
state machine
usage extraction
TTFB
TTFT
disconnect
cancel
backpressure
```

Forwarding prioritario.

---

## T8 — Usage normalization

Implementar:

```text
raw usage
normalized usage
consistency validation
normalizer_version
```

Con tests de:

```text
cached tokens
reasoning tokens
missing fields
invalid relations
overflow
```

---

## T9 — DAG integration

Relacionar:

```text
session
→ inference operation
→ provider request
→ provider attempt
→ provider usage
```

Sin huérfanos.

---

## T10 — Privacy + resource adversarial suite

Canary secrets.

Oversized input.

Malformed JSON.

Malformed SSE.

Observer overload.

Cancellation.

---

## T11 — Real/golden fixtures + backfill-ready parser

Crear corpus de fixtures.

Parser invocable offline.

Fragmentation/property tests.

Fuzz targets.

---

## T12 — CI + final verification

Añadir CI.

Ejecutar:

```text
F1 format
F2 clippy
F3 tests
F4 build
F5 privacy audit
F6 parser/fuzz smoke
F7 end-to-end Responses streaming
F8 independent review
```

---

# 51. Definition of Done

Phase 2 solo está cerrada cuando:

1. `/v1/chat/completions` mantiene el comportamiento Phase 1.

2. `/v1/responses` funciona transparentemente.

3. Request y response forwarding siguen siendo byte-exact cuando corresponda.

4. El semantic observer nunca bloquea el forwarding.

5. Podemos identificar provider/protocol/parser version.

6. Podemos reconstruir lifecycle del provider response.

7. Streaming se procesa incrementalmente.

8. Fragmentar SSE de forma distinta no cambia el resultado semántico.

9. `usage` raw queda preservado cuando existe.

10. `NormalizedUsage` es recalculable.

11. Cached tokens se distinguen de uncached.

12. Reasoning tokens se distinguen del resto del output cuando el provider los informa.

13. Missing usage produce `NULL/Unavailable`.

14. Stream cancelado no se registra como completed.

15. Parser failure no modifica response.

16. Observer backpressure no afecta TTFT significativamente.

17. Unknown provider fields no rompen parsing.

18. Inputs adversariales no provocan panic/OOM.

19. Secrets canary no aparecen en DB/logs/events.

20. Existe migration `0002`; `0001` permanece intacta.

21. Provider records están unidos al DAG causal.

22. Los 145+ tests Phase 0/1 siguen pasando.

23. CI ejecuta todos los gates obligatorios.

24. Existe documentación suficiente para implementar un segundo provider sin modificar el core.

25. No se ha introducido ninguna compresión real.

---

# 52. Independent review obligatorio

Antes de cerrar Phase 2, pedir a otra IA/reviewer que busque específicamente:

```text
semantic parsing en hot path
hidden reserialization
observer backpressure
memory growth in SSE
usage=0 when unknown
incorrect cached-token math
incorrect reasoning-token math
dangling DAG records
leaked secrets
migration corruption
parser-version ambiguity
stream cancellation bugs
incomplete → completed confusion
unbounded JSON/SSE
retry double counting
```

No considerar la fase terminada hasta resolver findings P0/P1.

---

# 53. Invariante final de Phase 2

Debe cumplirse:

```text
TRACEPRESS OFF
```

y:

```text
TRACEPRESS PHASE 2 ON
```

producen para la aplicación el mismo tráfico observable hacia/desde el provider, salvo latencia mínima de proxy.

La única diferencia debe ser que Tracepress ahora sabe:

```text
what request happened
what provider protocol was used
what lifecycle occurred
how many tokens were reported
how many were cached
how many were reasoning
whether that information is final/partial/unknown
how the operation relates causally to the session
```

Nada más.

---

# 54. Después de Phase 2

NO empezar directamente con adaptive compression.

El siguiente gate será:

```text
Phase 3 — Shadow Context Analysis
```

donde por primera vez analizaremos el contenido del request para responder:

```text
¿qué parte del input son tool results?
¿cuántos tokens ocupa cada bloque?
¿qué contenido es potencialmente comprimible?
¿cuánto habría ahorrado una transformación?
```

pero todavía sin modificar la request real.

Solo después de recopilar y validar esa información deberemos activar compresión.

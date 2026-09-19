# Phase 6.7 cache-key equality

Phase 6.7 observes cache-key presence and equality across the controlled Phase 6.6 arms without
persisting a key value or reusable key hash. It remains Shadow-only and does not modify an ordinary
Tracepress request.

## Single-process observer

One Tracepress proxy and one ephemeral equality observer remain alive for the complete 18-run
cohort. The observer sits on loopback between Codex and Tracepress. For every request it:

1. reads the already bounded request body;
2. records whether `prompt_cache_key` is absent, null, a string, or an invalid type;
3. compares a string value against values retained only in process memory;
4. assigns the first distinct value ordinal class `0`, the next class `1`, and so on; and
5. forwards the original body bytes unchanged to Tracepress.

The in-memory values are destroyed when the cohort child exits. Only status and ordinal equality
class enter temporary evidence. An ordinal has no stable meaning across processes and cannot be
used to reconstruct or compare a key outside its cohort.

The observer uses a streaming pass-through response and strips only hop-by-hop transport headers.
Contract tests require exact request-body forwarding and absence of key and prompt canaries from
the safe observer report.

## Arms and schedule

The Phase 6.6 arms and Williams schedule are retained:

```text
hooks_implicit_enabled
hooks_explicit_enabled
hooks_explicit_disabled
```

Every arm occupies every position twice. Two task blocks use the same pinned public ripgrep
checkout. Each run starts an ephemeral Codex session with the same model, read-only sandbox,
approval policy, and bounded Search task. The sole controlled difference is the `hooks` override.

## Gates

The certified result requires:

- 18/18 child sessions complete successfully;
- at least one observed provider request per child session, with the request-count distribution
  reported explicitly;
- a provider-usage row correlated to every equality record;
- no malformed or invalid cache-key observation; and
- no raw key, reusable hash, prompt, response, command, or authentication material in the report.

The classification is descriptive:

- `cache_key_absent`: every request omits the field;
- `one_shared_cache_key_class`: all requests use one equal value;
- `session_scoped_cache_key_classes`: both requests within each child session match, every child
  session has a distinct class, and no matched cross-arm round shares a class;
- `mixed_cache_key_classes`: keys are present but follow another equality pattern; or
- an incomplete-evidence classification when the execution, correlation, or presence gate fails.

Equality can explain a source of independent-session variation, but it does not prove the
provider's routing or cache-lookup algorithm and cannot authorize an instruction policy.

## Privacy boundary

The published report contains only aggregate status counts, the number of ordinal classes,
within-run equality, matched-round cross-arm equality counts, aggregate provider usage, safe arm
names, and execution gates. It contains no per-request class labels, key values, reusable hashes,
request bodies, fingerprints, paths, session identifiers, or request identifiers. All intermediate
evidence and the public checkout are deleted after aggregation.

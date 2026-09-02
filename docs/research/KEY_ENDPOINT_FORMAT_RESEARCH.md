# Key + Endpoint Request Format Research (Q3)

> Sources: 1mcp perplexity pplx_sonar + exa web_fetch/web_search, 2026-08-02.
> Provider pages fetched: build.nvidia.com/z-ai/glm-5.2, github.com/diegosouzapw/OmniRoute
> (docs/architecture/AUTHZ_GUIDE.md, src/sse/handlers/chat.ts, src/server/authz/pipeline.ts).

## Literal request header formats (the formats we must be idempotent across)

### 1. OpenAI (api.openai.com)

```http
POST https://api.openai.com/v1/chat/completions
Authorization: Bearer sk-...
Content-Type: application/json
{ "model": "gpt-4o", "messages": [...] }


```

- Auth value = gateway-side OpenAI key.
- Upstream endpoint = path + body.model.

### 2. NVIDIA build.nvidia.com GLM-5.2 (integrate.api.nvidia.com)

```python
from openai import OpenAI
client = OpenAI(base_url="https://integrate.api.nvidia.com/v1",
                api_key="$NVIDIA_API_KEY")
client.chat.completions.create(model="z-ai/glm-5.2", ...)


```

- Same OpenAI-compatible protocol. Authorization: Bearer $NVIDIA_API_KEY.
- Upstream endpoint identified by body.model = "z-ai/glm-5.2".

### 3. Anthropic (api.anthropic.com)

```text
POST https://api.anthropic.com/v1/messages
x-api-key: sk-ant-...
anthropic-version: 2023-06-01
{ "model": "claude-sonnet-5", "messages": [...] }


```

- NOT Bearer. x-api-key header carries the key.
- OmniRoute's validate route shows hybrid proxies send BOTH x-api-key and
  Bearer simultaneously; our normalize_auth handles both.

### 4. Azure OpenAI

```text
POST https://<resource>.openai.azure.com/openai/deployments/<id>/chat/completions?api-version=...
api-key: <azure-key>


```

- Custom api-key header, not Bearer.

## The identification mechanism (OmniRoute source-of-truth)

OmniRoute does NOT route by Authorization alone. Its pipeline:

```text
Incoming request -> src/proxy.ts
  -> runAuthzPipeline() in src/server/authz/pipeline.ts
     1. Strip trusted internal headers (x-omniroute-auth-*) from inbound
     2. extractApiKey(request) -> Authorization Bearer value  (client identity)
     3. classifyRoute()
     4. POLICIES[routeClass].evaluate(ctx)
        -> AuthSubject{ kind: client_api_key, id: key_<last-4>, scopes }
  -> handleChatCore()
     -> resolveModelOrError(body.model, body, endpoint, headers)
        -> { provider, model, sourceFormat, targetFormat }
     -> getExecutor(provider).execute() upstream
```

So the identity for "which (key, upstream endpoint) pair is this" is:

```text
(extractApiKey(request), body.model, request.path)
```

OmniRoute then stamps the downstream request with trusted internal headers
`x-omniroute-auth-id: key_<last-4>` etc., which downstream handlers can read
but which are stripped from inbound to prevent forgery.

## Resin-native gap

Resin v1.1.2 platform config uses:

- `reverse_proxy_fixed_account_header: "Authorization"` -> extracts auth value
  as the Account string.
- `regex_filters: ["\\.openai\\.com$"]` -> matches upstream REQUEST Host.

So Resin's (Platform, Account) pair = (upstream host, auth value). This is
distinct for different upstream hosts but ALIASES the same auth value across
different body.model on the same host (e.g. sk-abc calling openai/gpt-4 vs
openai/gpt-5.6 -> same Resin Account -> same egress IP by default). If the
project's promise is per-(key+endpoint) IP isolation, the body.model dimension
must be added on the shell side. That is what `route_id(auth, body_model, path)`
in crates/resin-core/src/lane.rs does.

## Proxy binding (OmniRoute priority order, for reference)

OmniRoute proxy resolution (PROXY_GUIDE.md) priority highest -> lowest:

1. account-level (per api key / OAuth connection)
2. provider-level (per provider, e.g. all OpenAI traffic)
3. combo-level (per combo/routing config)
4. global (all traffic, all providers)

First match wins. This mirrors what the project's Topology canvas B->C edge
(region_filters) achieves at the platform level, and what account-level proxy
binding achieves at the per-key level. The desktop shell's per-key candidate
drag-to-platform (P21) is the account-level equivalent.

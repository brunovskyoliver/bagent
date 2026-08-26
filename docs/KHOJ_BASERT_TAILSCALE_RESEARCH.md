# Khoj to BaseRT over Tailscale: configuration research

Date: 2026-07-27. This is research only; it makes no service or configuration
changes.

## Result

Khoj supports any OpenAI API-compatible server. BaseRT can therefore be used as
Khoj's OpenAI provider, provided the Khoj container can reach an OpenAI
`/v1/` endpoint on the Mac. Khoj does not need, configure, or load the model
itself: two clients may use one already-loaded BaseRT model, subject to the
server's available concurrency and memory.

`127.0.0.1:8082` cannot be used from the remote Ella host: on that host it
means Ella itself (and, from a container, its own network namespace). The
provider URL must instead resolve to a listener reachable over the Tailnet,
for example:

```text
http://<mac-tailnet-dns-name-or-100.x.y.z>:<separate-public-port>/v1/
```

Keep Bagnet pointed at its unchanged `http://127.0.0.1:8082/v1/` endpoint.
The second, Tailnet-reachable listener/proxy must protect the endpoint and
forward to the existing BaseRT service, or BaseRT must independently support a
second listener while sharing its loaded model. This is a BaseRT/network design
choice; Khoj merely needs the reachable OpenAI-compatible URL.

## Khoj configuration knobs

For an existing Khoj deployment, the admin UI path is least disruptive:

1. In **Admin -> AI Model API**, create an entry:
   - **Name:** arbitrary, e.g. `BaseRT over Tailnet`
   - **API key:** BaseRT's real bearer token; Khoj accepts any nonempty string
     in its generic setup, but a protected BaseRT endpoint should use its
     actual credential.
   - **API base URL:** the Tailnet URL above, including `/v1/`.
2. In **Admin -> Chat Model**, create/select:
   - **Name:** the exact ID returned by BaseRT `GET /v1/models` (expected to
     be the served Qwen model ID, not a display label)
   - **Model type:** `Openai`
   - **AI Model API:** the entry from step 1
   - **Max prompt size:** the model/server's actual usable context limit
   - **Tokenizer:** leave unset unless its behavior is known.
3. Select the model in the user chat settings. Also set it in Server Chat
   Settings **Default** and **Advanced** if Khoj should use it for internal
   intent/search steps.

For a fresh/rebuilt Compose deployment, the official Compose file provides
bootstrap variables:

```yaml
environment:
  - OPENAI_BASE_URL=http://<mac-tailnet-name-or-ip>:<port>/v1/
  - OPENAI_API_KEY=<Basert bearer token>
  - KHOJ_DEFAULT_CHAT_MODEL=<exact BaseRT /v1/models id>
```

The supplied Compose file is Docker syntax but its relevant environment values
are equally applicable to a Podman Compose deployment. Do not put a secret in
source control; use the existing secret/env mechanism on Ella.

## Verification sequence (after a later approved change)

1. From the Ella host and then from the Khoj **server container**, call
   `GET /v1/models` at the Tailnet URL with the bearer token. Verify the exact
   model ID and that no request is sent to the public internet.
2. Create the AI Model API and Chat Model in Khoj (or restart after changing
   bootstrap variables; Khoj uses `/v1/models` for discovery).
3. Send a short Khoj chat. Watch the BaseRT request log and confirm it reaches
   the existing loaded model, while Bagnet continues using 127.0.0.1:8082.
4. Exercise simultaneous brief requests from Khoj and Bagnet; tune the
   second listener/proxy/BaseRT queue only if latency or errors require it.

## Primary sources

- Khoj's [OpenAI Proxy guide](https://docs.khoj.dev/advanced/use-openai-proxy/)
  says it supports any OpenAI-compatible server and documents the AI Model API,
  Chat Model, `Openai` type, prompt size, tokenizer, and model-selection flow.
- Khoj's [self-hosting guide](https://docs.khoj.dev/get-started/setup/) documents
  `OPENAI_BASE_URL`, custom OpenAI-compatible providers, model discovery via a
  server restart, manual model addition, and Default/Advanced server chat
  settings for intermediate work.
- The official [Compose file](https://raw.githubusercontent.com/khoj-ai/khoj/master/docker-compose.yml)
  shows `OPENAI_BASE_URL`, `KHOJ_DEFAULT_CHAT_MODEL`, and the `/v1/` convention.
- The official [Khoj source](https://github.com/khoj-ai/khoj/blob/master/src/khoj/utils/initialization.py)
  creates an OpenAI client with `base_url`, lists `models`, and reads
  `OPENAI_BASE_URL` and `KHOJ_DEFAULT_CHAT_MODEL`.
- Khoj's [Tailscale guide](https://docs.khoj.dev/advanced/tailscale/) confirms
  the intended private cross-device service pattern. It documents exposing
  Khoj itself, not publishing a model server; the BaseRT listener/proxy remains
  an operator responsibility.

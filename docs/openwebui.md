# Connect Open WebUI to DecentraAI

Open WebUI is the **primary user-facing Chat**. DecentraAI's dashboard remains
the technical control-plane / admin UI — it is **not** replaced. This guide
wires Open WebUI to a DecentraAI node as an **OpenAI-compatible backend**, so
the user chats in Open WebUI while DecentraAI's autonomous fabric decides which
machine actually runs the inference.

No code changes are needed: DecentraAI already serves the standard OpenAI
surface that Open WebUI consumes.

## What Open WebUI needs

Open WebUI connects to any OpenAI-compatible endpoint that exposes:

- `GET  /v1/models`          → `{"object":"list","data":[{"id":...}]}`
- `POST /v1/chat/completions`→ `{"object":"chat.completion","choices":[{"message":{"content":...}}],"usage":{...}}`

DecentraAI provides exactly these out of the box (verified by the
`open_webui_openai_surface_round_trips_through_proxy` runtime test), both
streaming (SSE) and non-streaming.

## Step-by-step

1. Run the node (it exposes the API on `127.0.0.1:8080`):

   ```bash
   decentraai node --config ~/.decentraai/node.yaml
   # or via systemd:  systemctl --user status decentraai-node
   ```

2. Get an API token that Open WebUI will use (a subscription token, or the
   master token). The master token is at `~/.decentraai/runtime/api.token`; for
   a least-privilege seat create a subscription token:

   ```bash
   decentraai token create --name openwebui --tier 2   # prints dsk_<64hex> once
   ```

3. In Open WebUI → Settings → Connections → **+ Create a connection** →
   choose the **OpenAI** provider:
   - **URL**: `http://127.0.0.1:8080/v1`
   - **API Key**: the `dsk_...` token (or the master token)
   - **Model**: pick the served model id (e.g. `tinyllama.gguf` — list what
     `/v1/models` returns).

4. Start chatting. Open WebUI sends the request to DecentraAI, which
   authenticates the token, applies tier/rate-limit/role policy, and forwards
   the request through the fabric to the best available worker. The user does
   not see which machine runs it.

## Notes / security

- The node API binds to loopback (`127.0.0.1:8080`). If Open WebUI runs on a
  different machine, either run Open WebUI on the same host, or expose the API
  on the LAN deliberately (not recommended) and protect it with tokens + TLS.
- `api_auth_required: true` is on by default, so every Open WebUI request is
  authenticated; prompts and outputs are never logged.
- Tier allowlists (P1) and role separation (H4) apply to Open WebUI tokens like
  any other subscriber: a `client`/tier-1 token can only reach its allowed
  models and rate limits.
- The dashboard stays the control plane: watch `/` (http://127.0.0.1:8080/) for
  Workers / Network / Execution / Admin while users chat in Open WebUI.
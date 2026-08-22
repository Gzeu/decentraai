# DecentraAI Matrix Terminal

A lightweight client-facing terminal UI for the DecentraAI OpenAI-compatible API.

## Design

- cinematic Matrix-inspired background with modern glass/scanline effects
- terminal-first interaction
- key login accepting BOTH namespaces (`dca_…` consumer keys — what nodes
  issue today; `dsk_…` legacy subscription tokens), each PROBED against
  `GET /v1/models` before the login modal closes — a prefix alone is not
  authentication
- OpenAI-compatible chat completions, streaming
- in-flight request abort (`/stop` or the Abort button)
- bounded conversation history (last 24 messages) so prompts do not grow
  without limit on CPU nodes
- no tool/function-call advertisement: there is no client-side execution
  loop yet, and advertising tools the caller cannot run makes the backend
  burn context and the UI lie
- authenticated session state; the key lives ONLY in page memory (never
  localStorage, sessionStorage, or cookies)

## Endpoint

Same-origin by default (`v1`): serve this directory FROM the node (or any
proxy in front of it) and it works with zero configuration. To target another
node: `?api=https://host/v1`, or copy `config.example.json` to `config.json`
and set `apiBase`. No IP addresses are hardcoded in this client.

For production, prefer HTTPS and configure CORS/auth at the DecentraAI edge.

## Local preview

Serve the directory with any static HTTP server, for example:

```bash
python3 -m http.server 4173 --directory client/matrix-terminal
```

Then open `http://127.0.0.1:4173/?api=http://127.0.0.1:8080/v1`.

## Commands

- `/clear` — clears the terminal AND the conversation memory sent to the model
- `/stop` — aborts an in-flight request (slow CPU prefill can hold silence
  for minutes; keepalive comments keep the connection warm meanwhile)
- `/status` — connection + model + history size
- `/logout` — ends the session

## Security

The consumer key is held only in page memory for the active session. The
client never persists the key to localStorage, sessionStorage, or cookies.

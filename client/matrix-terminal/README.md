# DecentraAI Matrix Terminal

A lightweight client-facing terminal UI for the DecentraAI OpenAI-compatible API.

## Design

- cinematic Matrix-inspired background with modern glass/scanline effects
- terminal-first interaction
- API key login using `dsk_...` consumer keys
- OpenAI-compatible chat completions
- streaming responses
- visible tool/function-call events
- authenticated session state
- no model credentials embedded in the client

## Backend

The client targets an OpenAI-compatible endpoint configured at runtime. Default development endpoint:

`http://169.58.213.145/v1`

For production, prefer HTTPS and configure CORS/auth at the DecentraAI edge.

## Local preview

Serve the directory with any static HTTP server, for example:

```bash
python3 -m http.server 4173 --directory client/matrix-terminal
```

Then open `http://127.0.0.1:4173/`.

## Security

The consumer key is held only in page memory for the active session. The client never persists the key to localStorage, sessionStorage, or cookies.

# Exposure providers

Hydian does not bundle tunnel agents. Provider adapters detect executables and
show the command, scope, authentication facts, TLS facts, and limitations.

```text
hydian expose plan tailscale --scope tailnet
hydian expose start tailscale --scope public --dry-run
hydian expose plan ngrok -- --url example.ngrok.app
hydian expose plan cloudflare --mode quick
hydian expose plan cloudflare --mode existing -- tunnel run my-tunnel
hydian expose plan custom -- my-tunnel --upstream {local_url}
```

Tailscale `tailnet` maps to Serve and `public` maps to Funnel. Tailscale
terminates HTTPS; identity headers may exist in Serve mode. Funnel is public.

ngrok uses `ngrok http` and reads the assigned URL through the local Agent API.
Authentication tokens are rejected from Hydian provider arguments; configure
the ngrok agent credential store.

Cloudflare Quick Tunnel uses `cloudflared tunnel --url`. Cloudflare documents
Quick Tunnels as development-only and says they do not support SSE, so Hydian
marks this integration experimental and does not claim full MCP compatibility.
Named/existing tunnels accept provider-native arguments.

Custom placeholders are `{local_url}`, `{local_host}`, `{local_port}`, and
`{mcp_path}`. This is the escape hatch for SSH, reverse proxies, WARP/private
network arrangements, and future providers.

## OpenAI tunnel-client

Hydian does not embed OpenAI credentials or reproduce tunnel-client. Point the
installed OpenAI tunnel-client at:

```text
http://127.0.0.1:7337/mcp
```

`hydian endpoint --format openai` prints that upstream. Hydian intentionally
does not emit a guessed command because no stable public command surface was
confirmed during v0.1 research.

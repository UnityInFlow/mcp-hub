# Demo recording

The README references `docs/demo/demo.gif`, which is not recorded yet. Target length: about 20 seconds.

Use throwaway bash servers so the demo runs offline, and a scratch directory and socket so it does not touch a real daemon:

```sh
export MCP_HUB_SOCKET=$(mktemp -d)/mcp-hub.sock
cd "$(mktemp -d)"
```

Script (type each line; pause about 2 seconds after each):

```sh
mcp-hub init                          # add a server via the wizard
mcp-hub start --daemon
mcp-hub status
mcp-hub logs --follow                 # Ctrl+C after a few lines
mcp-hub gen-config --format claude
```

Optional extra beat: `kill <pid of one server>`, then `mcp-hub status` to show the restart count.

## Recording with vhs

Save as `demo.tape` and run `vhs demo.tape`:

```
Output docs/demo/demo.gif
Set FontSize 16
Set Width 1000
Set Height 500
Type "mcp-hub init" Enter
Sleep 8s
Type "mcp-hub start --daemon" Enter
Sleep 3s
Type "mcp-hub status" Enter
Sleep 2s
Type "mcp-hub logs --follow" Enter
Sleep 3s
Ctrl+C
Type "mcp-hub gen-config --format claude" Enter
Sleep 3s
```

(`init` is interactive; script its answers with further `Type`/`Enter` lines, or pre-write `mcp-hub.toml` and skip it.)

## Recording with asciinema

```sh
asciinema rec demo.cast
agg demo.cast docs/demo/demo.gif
```

Stop the daemon afterwards with `mcp-hub stop`.

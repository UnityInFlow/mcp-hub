#!/usr/bin/env bash
# Minimal MCP server that responds to JSON-RPC pings on stdin.
# stderr output simulates server logs.
echo "Ping responder started" >&2
while IFS= read -r line; do
    id=$(echo "$line" | python3 -c "import sys,json; print(json.load(sys.stdin).get('id', 0))" 2>/dev/null)
    if [ -n "$id" ] && [ "$id" != "0" ]; then
        echo "{\"jsonrpc\":\"2.0\",\"result\":{},\"id\":$id}"
    fi
    echo "Received request id=$id" >&2
done

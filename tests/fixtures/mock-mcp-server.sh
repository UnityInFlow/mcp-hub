#!/usr/bin/env bash
# Mock MCP server for introspection integration tests.
#
# Handles: initialize, notifications/initialized, tools/list, resources/list,
#          prompts/list, and ping.
#
# By default all capability families (tools, resources, prompts) are declared.
# Pass environment variable MOCK_SKIP_RESOURCES=1 to omit resources capability
# from the initialize response.
#
# Pass MOCK_TOOLS_ERROR=1 to return an error on tools/list.
# Pass MOCK_SILENT_LISTS=1 to never respond to list requests (tests timeout path).

while IFS= read -r line; do
    method=$(echo "$line" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('method',''))" 2>/dev/null)
    id=$(echo "$line" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('id',''))" 2>/dev/null)

    case "$method" in
        initialize)
            if [ "${MOCK_SKIP_RESOURCES:-0}" = "1" ]; then
                caps='{"tools":{},"prompts":{}}'
            else
                caps='{"tools":{},"resources":{},"prompts":{}}'
            fi
            echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":$caps,\"serverInfo\":{\"name\":\"mock\",\"version\":\"1.0\"}}}"
            ;;
        "notifications/initialized")
            # Fire-and-forget — no response expected.
            ;;
        "tools/list")
            if [ "${MOCK_SILENT_LISTS:-0}" = "1" ]; then
                # Never respond — tests the timeout path.
                continue
            fi
            if [ "${MOCK_TOOLS_ERROR:-0}" = "1" ]; then
                echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}"
            else
                echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"tools\":[{\"name\":\"search\",\"description\":\"Search the web\"},{\"name\":\"fetch\",\"description\":\"Fetch a URL\"}]}}"
            fi
            ;;
        "resources/list")
            if [ "${MOCK_SILENT_LISTS:-0}" = "1" ]; then
                continue
            fi
            echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"resources\":[{\"uri\":\"file:///data\",\"name\":\"data\"}]}}"
            ;;
        "prompts/list")
            if [ "${MOCK_SILENT_LISTS:-0}" = "1" ]; then
                continue
            fi
            echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"prompts\":[{\"name\":\"summarize\",\"description\":\"Summarize text\"}]}}"
            ;;
        ping)
            echo "{\"jsonrpc\":\"2.0\",\"result\":{},\"id\":$id}"
            ;;
        *)
            # Unknown method — respond with error so the caller doesn't hang.
            if [ -n "$id" ]; then
                echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Unknown method\"}}"
            fi
            ;;
    esac
done

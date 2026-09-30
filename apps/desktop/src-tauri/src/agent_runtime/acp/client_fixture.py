"""Deterministic ACP interoperability peer using real client files/processes."""
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
mode = sys.argv[2]
session = "client-services"
prompt = None
request_index = 0

def emit(value):
    print(json.dumps({"jsonrpc": "2.0", **value}, ensure_ascii=False), flush=True)

def reply(message, value):
    emit({"id": message["id"], "result": value})

def update(value):
    emit({"method": "session/update", "params": {"sessionId": session, "update": value}})

def rpc(method, params, expected_error=None, ident=None):
    global request_index
    request_index += 1
    ident = ident or f"client-{request_index}"
    emit({"id": ident, "method": method, "params": {"sessionId": session, **params}})
    response = json.loads(sys.stdin.readline())
    if response.get("method") == "session/cancel":
        reply(prompt, {"stopReason": "cancelled"})
        sys.exit(0)
    assert response.get("id") == ident, response
    if expected_error is not None:
        assert response["error"]["code"] == expected_error, response
        return response
    assert "error" not in response, response
    return response["result"]

for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        caps = message["params"]["clientCapabilities"]
        assert caps["terminal"] and caps["fs"]["readTextFile"] and caps["fs"]["writeTextFile"]
        reply(message, {"protocolVersion": 1, "agentCapabilities": {"mcpCapabilities": {"http": True, "sse": True}}})
    elif method == "session/new":
        assert pathlib.Path(message["params"]["cwd"]) == root
        update({"sessionUpdate": "available_commands_update", "availableCommands": [{"name": "context", "description": "Inspect native context"}]})
        reply(message, {"sessionId": session, "models": {"currentModelId": "native", "availableModels": [{"modelId": "native", "name": "Native"}]}})
    elif method == "session/prompt":
        prompt = message
        blocks = message["params"]["prompt"]
        if mode == "parallel_permission":
            emit({"id": "approval", "method": "session/request_permission", "params": {"sessionId":session, "toolCall":{"toolCallId":"native-read", "title":"Read fixture"}, "options":[{"optionId":"once", "kind":"allow_once", "name":"Allow once"}]}})
            # This RPC must be serviced while the user's permission is pending.
            rpc("fs/read_text_file", {"path": str(root / "missing.txt")}, -32002)
            approved = json.loads(sys.stdin.readline())
            assert approved["id"] == "approval" and approved["result"]["outcome"]["optionId"] == "once"
            update({"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":"Concurrent services verified"}})
            reply(prompt, {"stopReason":"end_turn"})
            continue
        if mode == "commands":
            if blocks[0]["text"] == "/context":
                assert len(blocks) == 1
                update({"sessionUpdate": "usage_update", "used": 1000, "size": 64000})
                reply(prompt, {"stopReason": "end_turn"})
                continue
            assert "PROJECT_CONTEXT_MUST_REACH_AGENT" in blocks[0]["text"]
        else:
            path = str(root / "created-中文.txt")
            rpc("fs/read_text_file", {"path": path}, -32002)
            content = "first\r\n中文🙂\nlast\n"
            write = {"path": path, "content": content}
            rpc("fs/write_text_file", write, ident="create-once")
            rpc("fs/write_text_file", write, ident="create-once")
            assert rpc("fs/read_text_file", {"path": path, "line": 2, "limit": 1})["content"] == "中文🙂\n"
            large = root / "large.log"
            large.write_bytes(("x" * (5 * 1024 * 1024) + "\ntail 中文\n").encode("utf-8"))
            assert rpc("fs/read_text_file", {"path": str(large), "line": 2, "limit": 1})["content"] == "tail 中文\n"
            rpc("fs/read_text_file", {"path": str(large)}, -32602)
            rpc("fs/write_text_file", {"sessionId": "other", "path": str(root / "wrong-session.txt"), "content": "denied"}, -32602)
            update({"sessionUpdate": "tool_call", "toolCallId": "native-edit", "title": "Create source file", "kind": "edit", "status": "completed", "content": [{"type": "diff", "path": path, "oldText": None, "newText": content}]})
            terminal = rpc("terminal/create", {"command": sys.executable, "args": ["-c", "import sys,time;sys.stdout.buffer.write(('中🙂'*100+'DONE\\n').encode());sys.stdout.buffer.flush();time.sleep(0.2)"], "outputByteLimit": 24})["terminalId"]
            update({"sessionUpdate": "tool_call", "toolCallId": "native-terminal", "title": "Run native command", "kind": "execute", "status": "in_progress", "content": [{"type": "terminal", "terminalId": terminal}]})
            # More than the old 512-request budget, without a multi-minute wait.
            for _ in range(600):
                rpc("terminal/output", {"terminalId": terminal})
            assert rpc("terminal/wait_for_exit", {"terminalId": terminal})["exitCode"] == 0
            output = rpc("terminal/output", {"terminalId": terminal})
            assert output["truncated"] and "DONE" in output["output"] and "\ufffd" not in output["output"]
            rpc("terminal/release", {"terminalId": terminal})
            rpc("terminal/output", {"terminalId": terminal}, -32602)
            update({"sessionUpdate": "tool_call_update", "toolCallId": "native-terminal", "status": "completed", "content": [{"type": "terminal", "terminalId": terminal}]})
            for index in range(600):
                update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": f"Progress {index}"}})
                update({"sessionUpdate": "tool_call", "toolCallId": f"read-{index}", "title": "Native read", "kind": "read", "status": "completed"})
            for used in [25600, 6400]:
                update({"sessionUpdate": "usage_update", "used": used, "size": 64000})
        update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Native peer completed"}})
        reply(prompt, {"stopReason": "end_turn"})

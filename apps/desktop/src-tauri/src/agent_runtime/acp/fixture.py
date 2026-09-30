"""Local protocol peer. No credentials, external agents or network required."""
import json
import sys
import time
import subprocess

mode = sys.argv[1]
session = "会话-Δ"
config = [{"id": "model-id", "category": "model", "type": "select", "name": "Model", "currentValue": "first", "options": [{"value": "first", "name": "First"}, {"value": "vendor/模型", "name": "Model"}]}]
if mode == "dependent":
    config = [
        {"id": "provider", "type": "select", "name": "Provider", "currentValue": "A", "options": [{"value": "A", "name": "A"}, {"value": "B", "name": "B"}]},
        {"id": "model", "category": "model", "type": "select", "name": "Model", "currentValue": "A1", "options": [{"value": "A1", "name": "A1"}]},
        {"id": "reasoning_effort", "type": "select", "name": "Reasoning", "currentValue": "low", "options": [{"value": "low", "name": "Low"}]},
    ]

def emit(value):
    # Deliberately split multibyte UTF-8 across writes.
    data = (json.dumps({"jsonrpc": "2.0", **value}, ensure_ascii=False) + "\n").encode()
    for offset in range(0, len(data), 7):
        sys.stdout.buffer.write(data[offset:offset + 7])
        sys.stdout.buffer.flush()

def reply(message, result):
    emit({"id": message["id"], "result": result})

def update(value):
    emit({"method": "session/update", "params": {"sessionId": session, "update": value}})

pending = None
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if mode == "record" and method:
        with open(sys.argv[2], "a", encoding="utf-8") as log:
            log.write(json.dumps(message, ensure_ascii=False) + "\n")
    if method == "initialize":
        assert message["params"]["clientCapabilities"]["terminal"] is True
        assert message["params"]["clientCapabilities"]["fs"] == {"readTextFile": True, "writeTextFile": True}
        reply(message, {"protocolVersion": 9 if mode == "bad_version" else 1, "agentCapabilities": {"promptCapabilities": {"image": False}}})
    elif method == "session/new":
        if mode == "auth":
            emit({"id": message["id"], "error": {"code": -32000, "message": "credential-secret-must-not-escape"}})
        elif mode == "legacy":
            reply(message, {"sessionId": session, "models": {"currentModelId": "first", "availableModels": [{"modelId": "first", "name": "First"}, {"modelId": "vendor/模型", "name": "Model"}]}})
        else:
            reply(message, {"sessionId": session, "configOptions": config})
    elif method == "session/set_model":
        assert mode == "legacy" and message["params"]["modelId"] == "vendor/模型"
        reply(message, {})
    elif method == "session/set_config_option":
        if mode == "dependent":
            ident, value = message["params"]["configId"], message["params"]["value"]
            option = next(option for option in config if option["id"] == ident)
            assert any(choice["value"] == value for choice in option["options"])
            option["currentValue"] = value
            if ident == "provider":
                assert value == "B"
                config[1].update(currentValue="B1", options=[{"value": name, "name": name} for name in ["B1", "B2"]])
            if ident == "model":
                assert value == "B2" and config[0]["currentValue"] == "B"
                config[2].update(currentValue="high", options=[{"value": "high", "name": "High"}, {"value":"default", "name":"Default"}])
            if ident == "reasoning_effort" and value == "default":
                option["currentValue"] = "high"
            reply(message, {"configOptions": config})
            continue
        if mode != "unconfirmed":
            config[0]["currentValue"] = message["params"]["value"]
        reply(message, {"configOptions": config})
    elif method == "session/prompt":
        pending = message
        if mode == "tree":
            subprocess.Popen([sys.executable, "-c", "import time,pathlib,sys;time.sleep(1);pathlib.Path(sys.argv[1]).write_text('orphan')", sys.argv[2]], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "child started"}})
            continue
        if mode == "eof":
            sys.exit(0)
        if mode == "hang":
            continue
        if mode == "wrong_session":
            emit({"method": "session/update", "params": {"sessionId": "wrong", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "bad"}}}})
            continue
        update({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "thinking-私密"}})
        update({"sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "Native read", "kind": "read", "status": "pending"})
        emit({"id": "permission-1", "method": "session/request_permission", "params": {"sessionId": session, "toolCall": {"toolCallId": "call-1", "title": "Native read", "rawInput": {"path": "safe-fixture"}}, "options": [{"optionId": "permanent", "kind": "allow_always", "name": "Always"}, {"optionId": "once", "kind": "allow_once", "name": "Once"}, {"optionId": "deny", "kind": "reject_once", "name": "Deny"}]}})
    elif method == "session/cancel":
        if pending:
            reply(pending, {"stopReason": "cancelled"})
        sys.exit(0)
    elif message.get("id") == "permission-1":
        selected = message["result"]["outcome"].get("optionId")
        assert selected != "permanent"
        if mode != "unfinished":
            update({"sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed" if selected == "once" else "failed", "content": [{"type": "content", "content": {"type": "text", "text": "native receipt"}}]})
        update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "结果 ✓ 日本語 العربية"}})
        reply(pending, {"stopReason": "max_tokens" if mode == "partial" else "end_turn"})

"""Exercise the compiled CLI and framed LSP, including real disk writes. No engine required."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
root = Path(__file__).resolve().parents[1]
env = os.environ.copy()
env.pop("LIGHTINGDAX_ADAPTER", None)
env.pop("LIGHTINGDAX_ADAPTER_ARGS", None)

def cli(*args):
    result = subprocess.run([binary, *map(str, args)], env=env, text=True, capture_output=True, timeout=20)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)

project = root / "examples/demo"
assert cli("inspect", project)["model"]["tables"][0]["name"] == "Sales"
assert cli("check", project) == {"diagnostics": [], "cycles": []}
result = cli("eval", project, "[Revenue]")
assert "result" not in result and result["source"] == "none" and result["warning"]

with tempfile.TemporaryDirectory() as temp:
    project = Path(temp) / "demo"
    shutil.copytree(root / "examples/demo", project)
    p = subprocess.Popen([binary, "lsp"], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    def send(method, params, id=None):
        request = {"jsonrpc": "2.0", "method": method, "params": params}
        if id is not None: request["id"] = id
        payload = json.dumps(request).encode()
        p.stdin.write(f"Content-Length: {len(payload)}\r\n\r\n".encode() + payload)
        p.stdin.flush()
    def receive():
        headers = {}
        while True:
            line = p.stdout.readline()
            assert line, p.stderr.read().decode()
            if line == b"\r\n": break
            name, value = line.decode().split(":", 1)
            headers[name.lower()] = value.strip()
        return json.loads(p.stdout.read(int(headers["content-length"])))
    try:
        send("initialize", {"initializationOptions": {"projectPath": str(project)}}, 1)
        assert receive()["result"]["capabilities"]["definitionProvider"]
        uri = "untitled:Revenue.dax"
        send("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "dax", "version": 1, "text": "SUM(Sales[Amount])"}})
        assert receive()["params"]["diagnostics"] == []
        send("lightingdax/bindMeasure", {"uri": uri, "table": "Sales", "name": "Revenue"}, 2)
        assert receive()["result"] is None
        send("textDocument/completion", {"textDocument": {"uri": uri}, "position": {"line": 0, "character": 1}}, 3)
        assert any(item["label"] == "[Revenue]" for item in receive()["result"]["items"])
        send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": "[Revenue]"}]})
        assert any("cycle" in d["message"].lower() for d in receive()["params"]["diagnostics"])
        send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": "SUM(Sales[Amount]) + 1"}]})
        assert receive()["params"]["diagnostics"] == []
        send("lightingdax/evaluate", {"uri": uri}, 4)
        result = receive()["result"]
        assert result["version"] == 3 and "result" not in result["evaluation"]
        send("lightingdax/saveMeasure", {"uri": uri}, 5)
        assert receive()["result"]["saved"]
        saved = (project / "Model.SemanticModel/definition/tables/Sales.tmdl").read_text()
        assert "measure Revenue = SUM(Sales[Amount]) + 1" in saved
        assert "lineageTag: 125ad1a2-63c8-4bd9-9811-f146bc06ea41" in saved
        # UTF-16 positions: the emoji occupies two units and the reference is resolved.
        send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 4}, "contentChanges": [{"text": '"😀" + [Units]'}]})
        receive()
        send("textDocument/definition", {"textDocument": {"uri": uri}, "position": {"line": 0, "character": 9}}, 6)
        assert receive()["result"]["uri"].endswith("Sales.tmdl")
        send("shutdown", {}, 7)
        assert receive()["result"] is None
        send("exit", {})
        assert p.wait(timeout=10) == 0
    finally:
        if p.poll() is None: p.kill()
print("CLI/LSP smoke checks passed")

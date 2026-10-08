"""Offline TOM/protocol smoke check, invoked after dotnet build. No Power BI needed."""
import json
from pathlib import Path
import subprocess
import sys
root = Path(__file__).resolve().parents[1]
requests = [
    {"id":1,"command":"load_model","params":{"path":str(root / "examples/demo/Model.SemanticModel/definition")}},
    {"id":2,"command":"unknown","params":{}},
    {"id":3,"command":"load_model","params":{"path":str(root / "examples/demo/Model.SemanticModel/definition")}},
]
p = subprocess.run(sys.argv[1:], input="\n".join(map(json.dumps, requests))+"\n", text=True, capture_output=True, timeout=30)
assert p.returncode == 0, p.stderr
responses = [json.loads(line) for line in p.stdout.splitlines()]
assert len(responses) == 3, p.stdout
assert responses[0]["result"]["tables"][0]["name"] == "Sales", responses[0]
assert responses[0]["result"]["tables"][0]["measures"][0]["name"] == "Revenue"
assert "error" in responses[1]
assert "result" in responses[2], "A failed request must not kill the adapter"
print("TOM/adapter protocol smoke checks passed")

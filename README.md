# Lightning DAX Workbench

A PBIP measure editor with a Rust language server, a VS Code results panel and a C# bridge to the Power BI engine. The repository name is `lightingdax`; the application is Lightning DAX.

This is an initial implementation of the editor and engine-backed workflow from the 8 October 2026 design. It also includes an experimental reference interpreter and an oracle-corpus recording/replay path. **It is not the proposed millisecond local engine yet.** No local function is marked verified and there are no performance claims.

## What works

- Read a semantic-model definition folder or a PBIP project folder containing one model. Inspect measures, columns and basic PBIR visual definitions.
- Open a measure in an editable DAX buffer. Get scalar-subset diagnostics, completion, measure definitions, conservative formatting and cycle detection.
- Save just the measure expression to TMDL, preserving other text, metadata and line endings. Stale file revisions refuse writes.
- Evaluate an expression or query on a Desktop/XMLA engine. Unsaved measure edits are applied with query-scoped `DEFINE MEASURE`, without modifying the live model.
- Display typed results with engine provenance. Disconnected, unverified expressions show a warning and **no number**.
- Preview explicit measure/column projections for basic cards, tables/matrices, bar/column and line visuals using Vega-Lite. Filters and interactions are outside this preview subset.
- Export Arrow table snapshots, record real scalar oracle cases and replay supported cases in Linux CI without an engine.

## Build and try without Power BI

Requirements: stable Rust, Node.js 22+, and .NET SDK 8 for the adapter.

```sh
cargo build --release
target/release/lightingdax inspect examples/demo
target/release/lightingdax check examples/demo
target/release/lightingdax eval examples/demo '[Revenue]'
```

The last command deliberately returns a disconnected warning. The example is a semantic-model/visual fixture, not a complete Power BI report package or a fabricated oracle corpus.

```sh
cd extension
npm ci
npm test
npm run package
```

Install the generated VSIX using VS Code's **Extensions: Install from VSIX** command. Open `examples/demo` as the workspace, set `lightingdax.serverPath` to the absolute Rust executable path, and run **DAX: Open Model Measure**. Use **DAX: Save Measure to TMDL** for model edits; normal VS Code Save acts on the DAX buffer.

For another project, open its workspace or set `lightingdax.projectPath`. If a folder contains several semantic models, select one `.SemanticModel/definition` folder explicitly. Restart the language server after settings, external TMDL edits or connection changes using **DAX: Reload Project and Connection**. Do not edit the model inside Desktop concurrently with these saves.

## Connect the real engine

```sh
dotnet publish adapter/LightingDax.Adapter.csproj -c Release -o adapter/publish
```

Set these environment variables before launching VS Code or the CLI. Keep connection strings out of tracked settings and source control.

```sh
export LIGHTINGDAX_ADAPTER=dotnet
export LIGHTINGDAX_ADAPTER_ARGS='["/absolute/path/adapter/publish/LightingDax.Adapter.dll"]'
export LIGHTINGDAX_CONNECTION='Data Source=localhost:YOUR_DESKTOP_PORT;Initial Catalog=YOUR_DATABASE;'
target/release/lightingdax eval /path/to/project '[Revenue]'
```

On Windows PowerShell:

```powershell
$env:LIGHTINGDAX_ADAPTER = 'dotnet'
$env:LIGHTINGDAX_ADAPTER_ARGS = '["C:/path/adapter/publish/LightingDax.Adapter.dll"]'
$env:LIGHTINGDAX_CONNECTION = 'Data Source=localhost:YOUR_DESKTOP_PORT;Initial Catalog=YOUR_DATABASE;'
```

Use an endpoint/catalog you already know from DAX Studio or your workspace. This implementation does not discover Desktop instances or provision tenant access. XMLA authentication follows the Microsoft client libraries and the supplied connection string. The endpoint must expose the same model as the project you are editing.

VS Code may alternatively configure `lightingdax.adapterProgram` and `lightingdax.adapterArgs`. The adapter inherits `LIGHTINGDAX_CONNECTION`. TOM model loading itself is offline; engine queries require a connection. A query is capped at 30 seconds and 100,000 result rows. Snapshot export streams batches and does not have the result-row cap.

## CLI commands

| Command | Purpose |
| --- | --- |
| `inspect PROJECT` | Model, source ranges, visuals and native-reader warnings as JSON |
| `check PROJECT` | Subset-parser/binder diagnostics and measure cycles; nonzero on findings |
| `eval PROJECT EXPRESSION` | Evaluate a scalar with `EVALUATE ROW` or return a disconnected warning |
| `query PROJECT QUERY` | Send a full DAX query to the engine |
| `save PROJECT TABLE MEASURE EXPRESSION` | Targeted TMDL expression save |
| `snapshot PROJECT TABLE OUTPUT.arrow` | Export a physical table; refuses to overwrite a file |
| `export-model PROJECT OUTPUT.json` | Export metadata for offline tests, using TOM when configured |
| `fingerprint MODEL.json SNAPSHOTS.json` | Compute the input fingerprint for an oracle corpus |
| `record PROJECT SNAPSHOTS.json CASES.json CORPUS.json` | Check snapshots against the engine and record scalar expected results |
| `test MODEL.json SNAPSHOTS.json CORPUS.json` | Replay recorded cases without starting an adapter |
| `lsp` | Run the framed stdio language server |

See [oracle testing](docs/oracle.md), [architecture and remaining phases](docs/architecture.md) and the [adapter protocol](docs/protocol.md).

## Validation

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --locked
python3 scripts/smoke.py target/debug/lightingdax
dotnet build adapter/LightingDax.Adapter.csproj -c Release
python3 scripts/adapter-smoke.py dotnet adapter/bin/Release/net8.0/LightingDax.Adapter.dll
cd extension && npm test && npm run package
```

CI runs Rust/CLI/LSP checks, extension tests/package creation, and offline adapter TOM checks on Linux and Windows. Live Desktop/XMLA queries still need validation on a connected machine; automated unit tests do not establish DAX correctness.

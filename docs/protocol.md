# Adapter and editor protocols

Adapter stdin/stdout is newline-delimited JSON, one request and response at a time. IDs are echoed exactly. stdout contains protocol messages only. The C# adapter stays alive after a failed request. Rust kills and reaps the child when its session closes.

```json
{"id":1,"command":"load_model","params":{"path":"/absolute/model/definition"}}
```

```json
{"id":1,"result":{"tables":[],"relationships":[]}}
```

```json
{"id":2,"error":{"code":"adapter_error","message":"Unknown adapter command."}}
```

| Command | Parameters | Result |
| --- | --- | --- |
| `load_model` | `path`: TOM definition folder | `Model` metadata matching `crates/core/src/model.rs` |
| `run_query` | `query`: DAX text | `path`, `rows`, `types`, `temporary: true`, `engine_version` |
| `snapshot_table` | `table`, absolute new `path` | Same Arrow descriptor with `temporary: false` |
| `list_functions` | `{}` | MDSCHEMA_FUNCTIONS rows with native metadata column names |
| `engine_info` | `{}` | `version` |

Data travels in Arrow IPC **file** format. Integer, boolean, floating-point and string columns use their corresponding Arrow types. Decimal and datetime values use invariant strings plus positional `types` metadata (`decimal`/`datetime`) so the bridge does not lose precision or roundtrip information. Null cells become DAX BLANK in the public result representation. Rust deletes temporary query files after decoding them; caller-owned snapshots remain on disk. Export refuses an existing path.

Public values are tagged JSON: `{"type":"blank"}`, `{"type":"number","value":1.5}`, or `{"type":"integer","value":"9007199254740993"}`. Integer payloads are strings because JSON/JavaScript numbers cannot represent all i64 values. Query column names remain exactly as returned by ADOMD.

The Rust language server uses standard `Content-Length` framing. It supports initialization, full text synchronization, completion, formatting, measure definitions, diagnostics and shutdown. Editor positions use UTF-16; core source ranges use byte offsets. Custom requests are `lightingdax/measures`, `bindMeasure`, `saveMeasure`, `evaluate` and `visuals`. Evaluation responses include the document version. The extension displays a result only if that version still matches the buffer.

Connection/authentication configuration is inherited through `LIGHTINGDAX_CONNECTION`; it is not sent over JSON. Initializers can supply `projectPath`, `adapterProgram` and an `adapterArgs` string array. Credentials should be supplied by the supported Microsoft client authentication mechanism, not embedded in tracked files.

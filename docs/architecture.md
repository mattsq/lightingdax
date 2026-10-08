# Architecture and coverage

The Rust process owns project text, the editor protocol, dependency analysis, result decoding, corpus comparison and the experimental reference interpreter. The C# child process owns TOM loading and all ADOMD engine access. The VS Code extension owns editable DAX buffers and a local-resource-only results webview.

## Delivered implementation

| Component | Implemented | Current boundary |
| --- | --- | --- |
| Project reader | Restricted native TMDL editing reader, TOM metadata hand-off, source revision hashes, targeted writes, PBIR visual discovery | One model per opened project; native reader does not load relationships or calculation groups; fenced expressions/multiline strings refuse saves |
| DAX front end | Lossless tokens, byte spans, recoverable scalar AST, reference binding, measure graph, cycles, reverse-dependency invalidation | No full DAX grammar, inferred types, variable scopes, rowan syntax tree or Salsa integration; `VAR` and query syntax may receive subset warnings |
| Evaluation routing | Engine queries, transient edited measure definitions, explicit source/status, disconnected refusal | No trusted local feature registry; engine results never promote functions to verified |
| Reference interpreter | Arithmetic, basic comparisons, BLANK, IF, DIVIDE, ISBLANK, aggregations, COUNTROWS, physical-table SUMX, explicit row/filter contexts, implicit measure context transition | Experimental; no CALCULATE arguments/modifiers, table expressions, relationship propagation, decimal/currency semantics, integer result inference or general DAX coercion |
| Snapshot store | Arrow IPC decoding, nullable typed rows, datetime/decimal type restoration, checked integer JSON, reproducible input fingerprints | Materializes row values; no mmap, dictionary kernels, indexes or parallel scans |
| Adapter | Long-lived NDJSON child, TOM, query/snapshot Arrow output, function metadata, engine version, per-request errors | Known endpoint and authentication supplied externally; no live-engine test executed in development |
| Interface | Stdio LSP, VS Code editing, results/cards/charts, grouped visual-query projection, CLI oracle replay | Basic projection previews only; matrix shown as grouped rows, not a pivot renderer |

Dependency analysis reuses cached analyses for unchanged measures. An update invalidates reverse-reachable measure dependents. This is an explicit small cache, not a claim of Salsa-level incremental parsing. Re-evaluation is debounced in the extension; engine requests are sequential, with queued document updates and version checks to reject stale displayed results. LSP cancellation is accepted but does not cancel an in-flight engine query.

All current local expressions are **unverified** or **unsupported**. A successful local unit test is not evidence against Power BI. The interpreter is only exposed to the oracle replay path; public evaluation routes to the real engine or produces a warning without a number. Exact-input corpus matches do not grant function-wide coverage.

The native reader makes editing possible before starting .NET, but it is deliberately restricted. TOM replaces its semantic metadata when an adapter is configured. Save mappings retain the native file revision, so TOM expression normalization cannot invalidate stale-write detection. File replacement is staged in the source directory; there is no multi-writer transaction or file watcher, so concurrent external writes must be avoided.

Preview queries use explicit measure/column projections and `SUMMARIZECOLUMNS` for grouped visuals. Unsupported projections, implicit aggregations, custom visual types and visual-local filters refuse mapping. **Page/report filters, slicers, visual interactions, formatting, sort and top-N settings are excluded.** The DAX preview query is opened in the editor for inspection. These are simplified projections, not report-equivalent results. Tables and cards preserve large integers; charts reject integer values outside JavaScript's exact range. Decimal charts use numeric plotting while the table preserves the decimal string.

## Next phase gates

1. Expand editor coverage: grammar reuse, variable binding, inferred types, TOM source maps for all legal TMDL measures, file watching, model selection and function-signature completion from the adapter.
2. Validate the thin client on real Desktop/XMLA models. Record real oracle cases and capture latency. Evaluate whether local execution materially improves the workflow compared with DAX Studio and Tabular Editor.
3. Implement the local semantics kernel and plan compiler: CALCULATE's five stages, modifiers, iterators/table expressions, relationship propagation, SUMMARIZECOLUMNS, shared grouped scans, Arrow mmap/dictionaries, DataFusion lowering and plan/interpreter/oracle differential checks. Function verification must require a meaningful engine corpus for both local tiers.
4. Add time intelligence, ALLSELECTED, calculation groups and richer relationship semantics only after the previous gate is supported by evidence.

The proposed under-50 ms diagnostics, under-100 ms ten-million-row evaluation and under-one-second startup targets have **not** been demonstrated. This branch makes the editor/adapter workflow testable and provides a safe starting point for those measurements.

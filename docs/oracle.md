# Oracle recording and offline replay

There is no engine-recorded corpus in this repository yet. Unit-test expectations are synthetic tests of the implementation, not Power BI verification.

1. Freeze a connected engine model and data. Keep the PBIP definition synchronized with that model. Export every required physical table with `snapshot` and preserve each returned `types` array. Do not refresh or modify the model between snapshot comparison and oracle recording.
2. Export TOM metadata with `export-model PROJECT model.json`. Offline CI must use that JSON when the model has relationships; the restricted native reader omits them.
3. Write a snapshot manifest with table names and Arrow descriptors. Paths are relative to the manifest, or absolute.

```json
{
  "Sales": {"path":"Sales.arrow","types":["number","integer","text"]}
}
```

4. Write a map of case names to scalar DAX expressions. Start with functions the reference interpreter implements.

```json
{
  "revenue": "[Revenue]",
  "blank addition": "BLANK() + 5.0"
}
```

5. Record and replay:

```sh
lightingdax record PROJECT snapshots.json cases.json corpus.json
lightingdax test model.json snapshots.json corpus.json
```

Recording compares each supplied table snapshot against a fresh engine query, including row order, types and blanks. A difference refuses recording; re-export rather than treating a mismatch as success. This is conservative: an engine row-order change can refuse a recording even if the underlying rows are equal. There is no transaction across the engine requests, so the endpoint must remain unchanged throughout recording.

The recorder obtains engine version from the connected server. The fingerprint covers semantic model JSON with editor source metadata removed, table column names, row order, types and data. The corpus stores the query, fingerprint, engine version and tagged expected result. Files are created only at new paths, so old evidence is not silently overwritten. Add those files to your governed test-data workflow; Arrow snapshots are ignored by Git by default.

The replay command starts no adapter, even if one is configured in the environment. It rejects empty corpora, missing engine versions and stale fingerprints. For each case it evaluates the experimental interpreter and compares column names, dimensions and tagged cell types, with a relative/absolute floating-point tolerance of `1e-9`. It fails rather than downgrading an unexpected value to success. Integers and floats differ; BLANK differs from zero; decimal and datetime representations remain distinct.

Expect early failures: literal/aggregate integer typing, currency precision, coercions, table expressions and relationship propagation are incomplete. Do not change expected results to match the interpreter. Preserve real failing cases for semantics work. The current corpus path is a regression scaffold, not a proof that every model property affecting DAX is captured by the prototype metadata projection.

Cases never promote all uses of a function to `verified`. Trusted local routing should only be enabled after plan/interpreter/oracle agreement and a corpus covering relevant context interactions, engine version changes and model features. Random query generation and tier disagreement downgrade/persistence are future work.

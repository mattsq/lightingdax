use crate::{
    analysis,
    model::Model,
    snapshot::{self, ResultTable, Store, Value},
    syntax::Expr,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Verified,
    Unverified,
    Unsupported,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Evaluation {
    pub source: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultTable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}
pub fn coverage(expression: &str, model: &Model) -> Status {
    fn visit(text: &str, model: &Model, seen: &mut BTreeSet<String>) -> Status {
        let a = analysis::analyze(text, model);
        if !a.diagnostics.is_empty() {
            return Status::Unsupported;
        }
        fn supported(e: &Expr) -> bool {
            match e {
                Expr::Call(n, args) => {
                    matches!(
                        n.as_str(),
                        "SUM"
                            | "AVERAGE"
                            | "MIN"
                            | "MAX"
                            | "COUNT"
                            | "COUNTROWS"
                            | "SUMX"
                            | "BLANK"
                            | "IF"
                            | "DIVIDE"
                            | "ISBLANK"
                            | "TRUE"
                            | "FALSE"
                    ) && args.iter().all(supported)
                }
                Expr::Unary(_, a) => supported(a),
                Expr::Binary(_, a, b) => supported(a) && supported(b),
                Expr::Error => false,
                _ => true,
            }
        }
        if !supported(&a.parsed.expression) {
            return Status::Unsupported;
        }
        for dependency in a.dependencies {
            if !seen.insert(dependency.clone()) {
                return Status::Unsupported;
            }
            if let Some((_, m)) = model.measure(&dependency) {
                if visit(&m.expression, model, seen) == Status::Unsupported {
                    return Status::Unsupported;
                }
            }
            seen.remove(&dependency);
        }
        Status::Unverified
    }
    visit(expression, model, &mut BTreeSet::new())
}
impl Evaluation {
    pub fn disconnected(expression: &str, model: &Model) -> Self {
        Self{source:"none".into(),status:coverage(expression,model),result:None,warning:Some("No engine connected. This expression has no engine-verified local coverage; no number is shown.".into())}
    }
    pub fn engine(result: ResultTable) -> Self {
        Self {
            source: "engine".into(),
            status: Status::Verified,
            result: Some(result),
            warning: None,
        }
    }
}
pub fn fingerprint(model: &Model, store: &Store) -> Result<String> {
    // Source file paths/ranges are editor metadata, not semantics.
    let mut model = model.clone();
    for t in &mut model.tables {
        for m in &mut t.measures {
            m.source = None;
        }
    }
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&model)?);
    hash.update(serde_json::to_vec(&store.tables)?);
    Ok(format!("{:x}", hash.finalize()))
}
#[derive(Debug, Serialize, Deserialize)]
pub struct OracleCase {
    pub name: String,
    pub expression: String,
    pub engine_version: String,
    pub fingerprint: String,
    pub expected: ResultTable,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Corpus {
    pub cases: Vec<OracleCase>,
}
/// Exact-input replay evidence only. It never promotes an entire function.
pub fn replay(corpus: &Corpus, model: &Model, store: &Store) -> Result<usize> {
    let fingerprint = fingerprint(model, store)?;
    if corpus.cases.is_empty() {
        bail!("Oracle corpus is empty");
    }
    for case in &corpus.cases {
        if case.engine_version.trim().is_empty() || case.fingerprint != fingerprint {
            bail!(
                "Case {} has missing engine provenance or stale model/snapshot fingerprint",
                case.name
            );
        }
        let value = crate::eval::Interpreter::new(model, store)
            .evaluate(&case.expression)
            .with_context(|| case.name.clone())?;
        let actual = ResultTable {
            columns: vec!["Value".into()],
            rows: vec![vec![value]],
        };
        snapshot::compare(&actual, &case.expected, 1e-9).with_context(|| case.name.clone())?;
    }
    Ok(corpus.cases.len())
}
pub fn scalar(value: Value) -> ResultTable {
    ResultTable {
        columns: vec!["Value".into()],
        rows: vec![vec![value]],
    }
}

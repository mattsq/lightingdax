use crate::{
    model::{same, Model},
    syntax::{self, Diagnostic, Expr, Parsed},
};
use std::collections::{BTreeMap, BTreeSet};

pub const FUNCTIONS: &[(&str, &str)] = &[
    ("SUM", "SUM(column)"),
    ("AVERAGE", "AVERAGE(column)"),
    ("MIN", "MIN(column)"),
    ("MAX", "MAX(column)"),
    ("COUNT", "COUNT(column)"),
    ("COUNTROWS", "COUNTROWS(table)"),
    ("SUMX", "SUMX(table, expression)"),
    ("BLANK", "BLANK()"),
    ("IF", "IF(condition, value, alternate)"),
    ("DIVIDE", "DIVIDE(numerator, denominator, alternate)"),
    ("CALCULATE", "CALCULATE(expression, filters...)"),
    ("FILTER", "FILTER(table, predicate)"),
    ("ALL", "ALL(tableOrColumn)"),
    ("REMOVEFILTERS", "REMOVEFILTERS(tableOrColumn)"),
    ("KEEPFILTERS", "KEEPFILTERS(filter)"),
    ("USERELATIONSHIP", "USERELATIONSHIP(column, column)"),
    (
        "SUMMARIZECOLUMNS",
        "SUMMARIZECOLUMNS(columns..., name, expression...)",
    ),
    ("VALUES", "VALUES(column)"),
    ("SELECTEDVALUE", "SELECTEDVALUE(column, alternate)"),
    ("ISBLANK", "ISBLANK(value)"),
    ("TRUE", "TRUE()"),
    ("FALSE", "FALSE()"),
];
#[derive(Debug, Clone)]
pub struct Analysis {
    pub parsed: Parsed,
    pub dependencies: BTreeSet<String>,
    pub functions: BTreeSet<String>,
    pub diagnostics: Vec<Diagnostic>,
}
pub fn analyze(text: &str, model: &Model) -> Analysis {
    let parsed = syntax::parse(text);
    let mut result = Analysis {
        diagnostics: parsed.diagnostics.clone(),
        parsed,
        dependencies: BTreeSet::new(),
        functions: BTreeSet::new(),
    };
    let expression = result.parsed.expression.clone();
    visit(&expression, model, &mut result);
    result
}
fn visit(e: &Expr, model: &Model, result: &mut Analysis) {
    match e {
        Expr::Ref { table, name } => {
            let resolved = model.qualified_measure(table.as_deref(), name);
            if let Some((_, m)) = resolved {
                result.dependencies.insert(m.name.clone());
            } else if !table
                .as_ref()
                .and_then(|t| model.table(t))
                .is_some_and(|t| t.columns.iter().any(|c| same(&c.name, name)))
            {
                result.diagnostics.push(Diagnostic {
                    start: 0,
                    end: 0,
                    message: format!(
                        "Unknown reference {}[{name}]",
                        table.as_deref().unwrap_or("")
                    ),
                });
            }
        }
        Expr::Table(name) => {
            if model.table(name).is_none() {
                result.diagnostics.push(Diagnostic{start:0,end:0,message:format!("Unknown table '{name}' (variables and query syntax require engine validation)")});
            }
        }
        Expr::Call(name, args) => {
            result.functions.insert(name.clone());
            for a in args {
                visit(a, model, result);
            }
        }
        Expr::Unary(_, a) => visit(a, model, result),
        Expr::Binary(_, a, b) => {
            visit(a, model, result);
            visit(b, model, result);
        }
        _ => {}
    }
}
/// Analysis of unchanged measures is reused. Only reverse-reachable dependents are invalidated.
#[derive(Default)]
pub struct Database {
    pub model: Model,
    pub revision: u64,
    cache: BTreeMap<String, Analysis>,
}
impl Database {
    pub fn new(model: Model) -> Self {
        Self {
            model,
            ..Default::default()
        }
    }
    pub fn analysis(&mut self, measure: &str) -> Option<&Analysis> {
        let name = self.model.measure(measure)?.1.name.clone();
        if !self.cache.contains_key(&name) {
            let expression = &self.model.measure(&name)?.1.expression;
            self.cache
                .insert(name.clone(), analyze(expression, &self.model));
        }
        self.cache.get(&name)
    }
    pub fn update(&mut self, measure: &str, expression: String) -> Option<BTreeSet<String>> {
        let names: Vec<_> = self
            .model
            .tables
            .iter()
            .flat_map(|t| t.measures.iter().map(|m| m.name.clone()))
            .collect();
        for name in &names {
            self.analysis(name);
        }
        let name = self.model.measure(measure)?.1.name.clone();
        let mut affected = BTreeSet::from([name.clone()]);
        loop {
            let previous = affected.len();
            for (n, a) in &self.cache {
                if a.dependencies.iter().any(|d| affected.contains(d)) {
                    affected.insert(n.clone());
                }
            }
            if previous == affected.len() {
                break;
            }
        }
        for table in &mut self.model.tables {
            if let Some(m) = table.measures.iter_mut().find(|m| same(&m.name, &name)) {
                m.expression = expression.clone();
            }
        }
        for name in &affected {
            self.cache.remove(name);
        }
        self.revision += 1;
        Some(affected)
    }
    pub fn cycles(&mut self) -> Vec<Vec<String>> {
        let names: Vec<_> = self
            .model
            .tables
            .iter()
            .flat_map(|t| t.measures.iter().map(|m| m.name.clone()))
            .collect();
        for n in &names {
            self.analysis(n);
        }
        fn dfs(
            name: &str,
            cache: &BTreeMap<String, Analysis>,
            path: &mut Vec<String>,
            done: &mut BTreeSet<String>,
            out: &mut Vec<Vec<String>>,
        ) {
            if let Some(i) = path.iter().position(|n| n == name) {
                let mut cycle = path[i..].to_vec();
                cycle.push(name.into());
                out.push(cycle);
                return;
            }
            if done.contains(name) {
                return;
            }
            path.push(name.into());
            if let Some(a) = cache.get(name) {
                for d in &a.dependencies {
                    dfs(d, cache, path, done, out);
                }
            }
            path.pop();
            done.insert(name.into());
        }
        let mut out = Vec::new();
        let mut done = BTreeSet::new();
        for n in names {
            dfs(&n, &self.cache, &mut Vec::new(), &mut done, &mut out);
        }
        out
    }
}

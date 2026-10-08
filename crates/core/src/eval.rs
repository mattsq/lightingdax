//! Experimental reference interpreter. It deliberately rejects semantics it cannot
//! model. Passing unit tests does not grant engine-verified status.
use crate::{
    model::{same, Model},
    snapshot::{Store, Value},
    syntax::{self, Expr},
};
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct FilterContext {
    pub columns: BTreeMap<(String, String), Vec<Value>>,
}
#[derive(Debug, Clone, Default)]
pub struct RowContext {
    pub rows: Vec<(String, usize)>,
}
pub struct Interpreter<'a> {
    pub model: &'a Model,
    pub store: &'a Store,
    pub filters: FilterContext,
    pub row: RowContext,
    stack: Vec<String>,
}
impl<'a> Interpreter<'a> {
    pub fn new(model: &'a Model, store: &'a Store) -> Self {
        Self {
            model,
            store,
            filters: FilterContext::default(),
            row: RowContext::default(),
            stack: Vec::new(),
        }
    }
    pub fn evaluate(&mut self, text: &str) -> Result<Value> {
        let p = syntax::parse(text);
        if !p.diagnostics.is_empty() {
            bail!("Unsupported or invalid expression: {:?}", p.diagnostics);
        }
        self.expr(&p.expression)
    }
    fn table(&self, name: &str) -> Result<&crate::snapshot::ResultTable> {
        self.store
            .tables
            .iter()
            .find(|(n, _)| same(n, name))
            .map(|(_, t)| t)
            .context("Table snapshot is missing")
    }
    fn column(&self, table: &str, column: &str, row: usize) -> Result<Value> {
        let t = self.table(table)?;
        let c = t
            .columns
            .iter()
            .position(|n| {
                same(n, column)
                    || n.to_lowercase()
                        .ends_with(&format!("[{}]", column.replace(']', "]]")).to_lowercase())
            })
            .context("Column snapshot is missing")?;
        t.rows
            .get(row)
            .and_then(|r| r.get(c))
            .cloned()
            .context("Snapshot row is missing")
    }
    fn surviving(&self, table: &str) -> Result<Vec<usize>> {
        if !self.model.relationships.is_empty() {
            bail!("Relationship propagation is not yet implemented in the reference interpreter");
        }
        let t = self.table(table)?;
        let mut result = Vec::new();
        for row in 0..t.rows.len() {
            let mut keep = true;
            for ((tn, cn), allowed) in &self.filters.columns {
                if same(tn, table) && !allowed.contains(&self.column(table, cn, row)?) {
                    keep = false;
                    break;
                }
            }
            if keep {
                result.push(row);
            }
        }
        Ok(result)
    }
    fn measure(&mut self, table: Option<&str>, name: &str) -> Result<Value> {
        let (_, m) = self
            .model
            .qualified_measure(table, name)
            .context("Unknown measure")?;
        if self.stack.len() >= 64 || self.stack.iter().any(|n| same(n, name)) {
            bail!("Cyclic or excessively nested measure reference: {name}");
        }
        let expression = m.expression.clone();
        let outer = self.filters.clone();
        // Implicit CALCULATE on a measure called within row context.
        let mut transition = Vec::new();
        for (table, row) in &self.row.rows {
            let t = self.table(table)?;
            for (column, value) in t.columns.iter().zip(&t.rows[*row]) {
                transition.push((
                    (table.to_lowercase(), column.to_lowercase()),
                    vec![value.clone()],
                ));
            }
        }
        for (column, values) in transition {
            self.filters.columns.insert(column, values);
        }
        self.stack.push(name.into());
        let result = self.evaluate(&expression);
        self.stack.pop();
        self.filters = outer;
        result
    }
    fn expr(&mut self, e: &Expr) -> Result<Value> {
        match e {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::Text(s) => Ok(Value::Text(s.clone())),
            Expr::Boolean(b) => Ok(Value::Boolean(*b)),
            Expr::Ref { table, name } => {
                if self
                    .model
                    .qualified_measure(table.as_deref(), name)
                    .is_some()
                {
                    return self.measure(table.as_deref(), name);
                }
                let table = table.as_ref().context("Unqualified column reference")?;
                let (_, row) = self
                    .row
                    .rows
                    .iter()
                    .rev()
                    .find(|(t, _)| same(t, table))
                    .context("Column needs row context")?;
                self.column(table, name, *row)
            }
            Expr::Unary(op, a) => {
                let n = self.expr(a)?.number()?;
                Ok(Value::Number(if op == "-" { -n } else { n }))
            }
            Expr::Binary(op, a, b) => {
                let a = self.expr(a)?;
                let b = self.expr(b)?;
                if op == "==" {
                    return Ok(Value::Boolean(a == b));
                }
                if op == "&&" || op == "||" {
                    return Ok(Value::Boolean(if op == "&&" {
                        a.truth()? && b.truth()?
                    } else {
                        a.truth()? || b.truth()?
                    }));
                }
                if matches!(op.as_str(), "=" | "<>") {
                    let eq = if matches!((&a, &b), (Value::Text(_), Value::Text(_))) {
                        a == b
                    } else {
                        a.number()? == b.number()?
                    };
                    return Ok(Value::Boolean(if op == "=" { eq } else { !eq }));
                }
                let both_blank = a == Value::Blank && b == Value::Blank;
                let x = a.number()?;
                let y = b.number()?;
                Ok(match op.as_str() {
                    "+" if both_blank => Value::Blank,
                    "+" => Value::Number(x + y),
                    "-" => Value::Number(x - y),
                    "*" if a == Value::Blank || b == Value::Blank => Value::Blank,
                    "*" => Value::Number(x * y),
                    "/" if y == 0.0 => bail!("Non-finite division requires engine evaluation"),
                    "/" => Value::Number(x / y),
                    "^" => Value::Number(x.powf(y)),
                    "<" => Value::Boolean(x < y),
                    ">" => Value::Boolean(x > y),
                    "<=" => Value::Boolean(x <= y),
                    ">=" => Value::Boolean(x >= y),
                    _ => bail!("Unsupported operator {op}"),
                })
            }
            Expr::Call(name, args) => self.call(name, args),
            _ => bail!("Table or incomplete expression used as a scalar"),
        }
    }
    fn call(&mut self, name: &str, args: &[Expr]) -> Result<Value> {
        match name {
            "BLANK" if args.is_empty() => Ok(Value::Blank),
            "TRUE" if args.is_empty() => Ok(Value::Boolean(true)),
            "FALSE" if args.is_empty() => Ok(Value::Boolean(false)),
            "ISBLANK" if args.len() == 1 => {
                Ok(Value::Boolean(self.expr(&args[0])? == Value::Blank))
            }
            "IF" if (2..=3).contains(&args.len()) => {
                if self.expr(&args[0])?.truth()? {
                    self.expr(&args[1])
                } else if args.len() == 3 {
                    self.expr(&args[2])
                } else {
                    Ok(Value::Blank)
                }
            }
            "DIVIDE" if (2..=3).contains(&args.len()) => {
                let a = self.expr(&args[0])?;
                let b = self.expr(&args[1])?.number()?;
                if b == 0.0 {
                    if args.len() == 3 {
                        self.expr(&args[2])
                    } else {
                        Ok(Value::Blank)
                    }
                } else if a == Value::Blank {
                    Ok(Value::Blank)
                } else {
                    Ok(Value::Number(a.number()? / b))
                }
            }
            "COUNTROWS" if args.len() == 1 => {
                let table = match &args[0] {
                    Expr::Table(t) => t,
                    _ => bail!("COUNTROWS requires a physical table"),
                };
                let rows = self.surviving(table)?;
                if rows.is_empty() {
                    Ok(Value::Blank)
                } else {
                    Ok(Value::Integer(rows.len() as i64))
                }
            }
            "SUM" | "AVERAGE" | "MIN" | "MAX" | "COUNT" if args.len() == 1 => {
                let (table, column) = match &args[0] {
                    Expr::Ref {
                        table: Some(t),
                        name,
                    } => (t, name),
                    _ => bail!("Aggregation requires a qualified column"),
                };
                let mut values = Vec::new();
                for row in self.surviving(table)? {
                    let v = self.column(table, column, row)?;
                    if v != Value::Blank {
                        values.push(v.number()?);
                    }
                }
                if values.is_empty() {
                    return Ok(Value::Blank);
                }
                Ok(match name {
                    "COUNT" => Value::Integer(values.len() as i64),
                    "SUM" => Value::Number(values.iter().sum()),
                    "AVERAGE" => Value::Number(values.iter().sum::<f64>() / values.len() as f64),
                    "MIN" => Value::Number(values.into_iter().fold(f64::INFINITY, f64::min)),
                    _ => Value::Number(values.into_iter().fold(f64::NEG_INFINITY, f64::max)),
                })
            }
            "SUMX" if args.len() == 2 => {
                let table = match &args[0] {
                    Expr::Table(t) => t,
                    _ => bail!("SUMX currently requires a physical table"),
                };
                let mut sum = 0.0;
                let mut any = false;
                for row in self.surviving(table)? {
                    self.row.rows.push((table.clone(), row));
                    let value = self.expr(&args[1]);
                    self.row.rows.pop();
                    let v = value?;
                    if v != Value::Blank {
                        any = true;
                        sum += v.number()?;
                    }
                }
                Ok(if any {
                    Value::Number(sum)
                } else {
                    Value::Blank
                })
            }
            _ => bail!("Function {name} or its argument shape is not implemented locally"),
        }
    }
}

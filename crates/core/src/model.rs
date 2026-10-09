use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub tables: Vec<Table>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    #[serde(default)]
    pub columns: Vec<Column>,
    #[serde(default)]
    pub measures: Vec<Measure>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    #[serde(default)]
    pub data_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measure {
    pub name: String,
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    pub active: bool,
    pub single_direction: bool,
}
pub fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}
impl Model {
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| same(&t.name, name))
    }
    pub fn measure(&self, name: &str) -> Option<(&Table, &Measure)> {
        self.tables.iter().find_map(|t| {
            t.measures
                .iter()
                .find(|m| same(&m.name, name))
                .map(|m| (t, m))
        })
    }
    pub fn qualified_measure(&self, table: Option<&str>, name: &str) -> Option<(&Table, &Measure)> {
        match table {
            Some(t) => self.table(t).and_then(|t| {
                t.measures
                    .iter()
                    .find(|m| same(&m.name, name))
                    .map(|m| (t, m))
            }),
            None => self.measure(name),
        }
    }
}

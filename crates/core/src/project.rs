use crate::model::*;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Visual {
    pub path: PathBuf,
    pub kind: String,
    pub definition: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub root: PathBuf,
    pub definition: PathBuf,
    pub model: Model,
    pub visuals: Vec<Visual>,
    pub warnings: Vec<String>,
}
pub fn files(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(root, extension, &mut out)?;
    out.sort();
    Ok(out)
}
fn walk(root: &Path, extension: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some(".git" | "node_modules" | "target" | ".lightingdax")
            ) {
                walk(&path, extension, out)?;
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            out.push(path);
        }
    }
    Ok(())
}
fn name(text: &str) -> String {
    text.trim().trim_matches('\'').replace("''", "'")
}
fn metadata(text: &str) -> bool {
    [
        "formatString:",
        "displayFolder:",
        "description:",
        "lineageTag:",
        "isHidden",
        "annotation ",
        "changedProperty ",
        "extendedProperty ",
        "dataCategory:",
        "detailRowsDefinition",
        "formatStringDefinition",
    ]
    .iter()
    .any(|p| text.starts_with(p))
}
/// Restricted native reader for editing without Windows. TOM remains authoritative.
pub fn read_table(path: &Path) -> Result<Table> {
    let text = fs::read_to_string(path)?;
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        lines.push((offset, line.trim_end_matches(['\r', '\n'])));
        offset += line.len();
    }
    let table_name = lines
        .iter()
        .find_map(|(_, l)| l.strip_prefix("table "))
        .context("Missing table declaration")?;
    let mut table = Table {
        name: name(table_name),
        ..Default::default()
    };
    let mut i = 0;
    while i < lines.len() {
        let (base, line) = lines[i];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(column) = trimmed.strip_prefix("column ") {
            let mut data_type = String::new();
            let mut j = i + 1;
            while j < lines.len() {
                let l = lines[j].1;
                let t = l.trim_start();
                if !t.is_empty() && l.len() - t.len() <= indent {
                    break;
                }
                if let Some(x) = t.strip_prefix("dataType:") {
                    data_type = x.trim().into();
                }
                j += 1;
            }
            table.columns.push(Column {
                name: name(column.split('=').next().unwrap()),
                data_type,
            });
        }
        if let Some(declaration) = trimmed.strip_prefix("measure ") {
            // '=' may occur in a quoted name, so find the closing quote first.
            let name_end = if declaration.starts_with('\'') {
                let mut at = 1;
                let bytes = declaration.as_bytes();
                while at < bytes.len() {
                    if bytes[at] == b'\'' {
                        if at + 1 < bytes.len() && bytes[at + 1] == b'\'' {
                            at += 2;
                        } else {
                            at += 1;
                            break;
                        }
                    } else {
                        at += 1;
                    }
                }
                at
            } else {
                declaration.find('=').context("Measure needs '='")?
            };
            let eq = name_end
                + declaration[name_end..]
                    .find('=')
                    .context("Measure needs '='")?;
            let measure_name = name(&declaration[..eq]);
            let start = base + indent + "measure ".len() + eq + 1;
            let mut end = base + line.len();
            let mut j = i + 1;
            while j < lines.len() {
                let (b, l) = lines[j];
                let t = l.trim_start();
                if !t.is_empty() && (l.len() - t.len() <= indent || metadata(t)) {
                    break;
                }
                if !t.is_empty() {
                    end = b + l.len();
                }
                j += 1;
            }
            let expression = text[start..end].trim().to_string();
            if expression.starts_with("```") {
                bail!(
                    "Fenced TMDL expression in {} requires TOM source mapping",
                    path.display()
                );
            }
            table.measures.push(Measure {
                name: measure_name,
                expression,
                source: Some(Source {
                    path: path.to_path_buf(),
                    start,
                    end,
                    revision: format!("{:x}", Sha256::digest(text.as_bytes())),
                }),
            });
            i = j;
            continue;
        }
        i += 1;
    }
    Ok(table)
}
impl Project {
    pub fn open(input: &Path) -> Result<Self> {
        let input = input
            .canonicalize()
            .context("Project path does not exist")?;
        let root = if input.is_file() {
            input.parent().context("No project folder")?.to_path_buf()
        } else {
            input
        };
        let mut definitions = Vec::new();
        if root.join("tables").is_dir() {
            definitions.push(root.clone());
        } else {
            for path in files(&root, "tmdl")? {
                if path.file_name().and_then(|x| x.to_str()) == Some("model.tmdl") {
                    definitions.push(path.parent().unwrap().to_path_buf());
                }
            }
        }
        if definitions.len() != 1 {
            bail!("Expected one semantic model definition, found {}. Open its .SemanticModel/definition folder when a workspace has several models.",definitions.len());
        }
        let definition = definitions.remove(0);
        let mut tables = Vec::new();
        for path in files(&definition.join("tables"), "tmdl")? {
            tables.push(read_table(&path)?);
        }
        let model = Model {
            tables,
            relationships: Vec::new(),
        };
        let mut visuals = Vec::new();
        for path in files(&root, "json")? {
            if path.file_name().and_then(|x| x.to_str()) != Some("visual.json") {
                continue;
            }
            let data: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            let kind = data
                .pointer("/visual/visualType")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_owned();
            visuals.push(Visual {
                path,
                kind,
                definition: data,
            });
        }
        Ok(Self{root,definition,model,visuals,warnings:vec!["Restricted native TMDL reader: relationships, calculation groups and model properties require the TOM adapter. Local results are unverified.".into()]})
    }
    pub fn apply_authoritative_model(&mut self, mut model: Model) {
        for table in &mut model.tables {
            for measure in &mut table.measures {
                measure.source = self
                    .model
                    .qualified_measure(Some(&table.name), &measure.name)
                    .and_then(|(_, m)| m.source.clone());
            }
        }
        self.model = model;
        self.warnings.clear();
    }
    pub fn write_measure(&self, table: &str, measure: &str, expression: &str) -> Result<()> {
        let (tokens, _) = crate::syntax::lex(expression);
        if tokens.iter().any(|t|matches!(&t.kind,crate::syntax::Kind::Text(s) if s.contains('\n') || s.contains('\r'))) {
            bail!("Multiline string literals require a lossless TMDL writer; this save is not supported");
        }
        let (_, m) = self
            .model
            .qualified_measure(Some(table), measure)
            .context("Unknown measure")?;
        let source = m
            .source
            .as_ref()
            .context("No safe text source mapping for this measure")?;
        let current = fs::read_to_string(&source.path)?;
        if source.revision.is_empty()
            || format!("{:x}", Sha256::digest(current.as_bytes())) != source.revision
            || current.get(source.start..source.end).is_none()
        {
            bail!("TMDL changed since loading; reload before saving");
        }
        let before = &current[..source.start];
        let line = before.rsplit('\n').next().unwrap_or("");
        let indent: String = line
            .chars()
            .take_while(|c| *c == '\t' || *c == ' ')
            .collect();
        let newline = if current.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let replacement = if expression.contains('\n') {
            format!(
                "{newline}{}\t{}",
                indent,
                expression
                    .lines()
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(&format!("{newline}{indent}\t"))
            )
        } else {
            format!(" {}", expression.trim())
        };
        let updated = format!("{}{}{}", before, replacement, &current[source.end..]);
        // create_new avoids overwriting another writer's temporary file.
        let temporary = source
            .path
            .with_extension(format!("tmdl.{}.tmp", std::process::id()));
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| -> Result<()> {
            file.write_all(updated.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &source.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

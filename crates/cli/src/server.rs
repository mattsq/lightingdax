//! Minimal stdio LSP transport. Positions are UTF-16, core spans are UTF-8 bytes.
use anyhow::{bail, Context, Result};
use lightingdax_core::{
    adapter::Adapter,
    analysis::{self, Database},
    model::Model,
    project::Project,
    syntax,
    trust::Evaluation,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Read, Write},
    path::Path,
};

struct Document {
    text: String,
    version: i64,
    binding: Option<(String, String)>,
}
struct Server {
    project: Option<Project>,
    db: Database,
    engine: Option<Adapter>,
    documents: BTreeMap<String, Document>,
}
fn position(text: &str, byte: usize) -> Value {
    let mut safe = byte.min(text.len());
    while !text.is_char_boundary(safe) {
        safe -= 1;
    }
    let prefix = &text[..safe];
    let line = prefix.bytes().filter(|b| *b == b'\n').count();
    let character = prefix
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .count();
    json!({"line":line,"character":character})
}
fn range(text: &str, start: usize, end: usize) -> Value {
    json!({"start":position(text,start),"end":position(text,end)})
}
fn write(output: &mut impl Write, message: Value) -> Result<()> {
    let bytes = serde_json::to_vec(&message)?;
    write!(output, "Content-Length: {}\r\n\r\n", bytes.len())?;
    output.write_all(&bytes)?;
    output.flush()?;
    Ok(())
}
pub fn run() -> Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut shutdown = false;
    let mut server = Server {
        project: None,
        db: Database::new(Model::default()),
        engine: None,
        documents: BTreeMap::new(),
    };
    loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                return Ok(());
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = Some(value.trim().parse::<usize>()?);
            }
        }
        let length = length.context("Missing Content-Length")?;
        if length > 8 * 1024 * 1024 {
            bail!("LSP message too large");
        }
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes)?;
        let request: Value = serde_json::from_slice(&bytes)?;
        let method = request["method"].as_str().unwrap_or("");
        let params = &request["params"];
        let id = request.get("id");
        if method == "exit" {
            return if shutdown {
                Ok(())
            } else {
                bail!("LSP exited without shutdown")
            };
        }
        if method == "shutdown" {
            shutdown = true;
            if let Some(id) = id {
                write(&mut output, json!({"jsonrpc":"2.0","id":id,"result":null}))?;
            }
            continue;
        }
        if shutdown {
            if let Some(id) = id {
                write(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"Server is shutting down"}}),
                )?;
            }
            continue;
        }
        match server.handle(method, params, &mut output) {
            Ok(result) => {
                if let Some(id) = id {
                    write(
                        &mut output,
                        json!({"jsonrpc":"2.0","id":id,"result":result}),
                    )?;
                }
            }
            Err(e) => {
                if let Some(id) = id {
                    write(
                        &mut output,
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":format!("{e:#}")}}),
                    )?;
                } else {
                    write(
                        &mut output,
                        json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"type":1,"message":format!("{e:#}")}}),
                    )?;
                }
            }
        }
    }
}
impl Server {
    fn handle(&mut self, method: &str, params: &Value, output: &mut impl Write) -> Result<Value> {
        match method {
            "initialize" => {
                let options = &params["initializationOptions"];
                if let Some(program) = options["adapterProgram"].as_str().filter(|s| !s.is_empty())
                {
                    let args: Vec<String> = serde_json::from_value(
                        options.get("adapterArgs").cloned().unwrap_or(json!([])),
                    )?;
                    self.engine = Some(Adapter::start(program, &args)?);
                } else {
                    self.engine = super::adapter()?;
                }
                if let Some(path) = options["projectPath"].as_str().filter(|s| !s.is_empty()) {
                    let mut project = Project::open(Path::new(path))?;
                    if let Some(engine) = &mut self.engine {
                        project.apply_authoritative_model(engine.load_model(&project.definition)?);
                    }
                    self.db = Database::new(project.model.clone());
                    self.project = Some(project);
                }
                Ok(
                    json!({"capabilities":{"textDocumentSync":1,"completionProvider":{"triggerCharacters":["[","'","("]},"definitionProvider":true,"documentFormattingProvider":true},"serverInfo":{"name":"lightingdax","version":env!("CARGO_PKG_VERSION")}}),
                )
            }
            "initialized" | "$/cancelRequest" | "$/setTrace" => Ok(Value::Null),
            "textDocument/didOpen" | "textDocument/didChange" => {
                let td = &params["textDocument"];
                let uri = td["uri"].as_str().context("Missing URI")?.to_owned();
                let text = if method.ends_with("didOpen") {
                    td["text"].as_str()
                } else {
                    params["contentChanges"][0]["text"].as_str()
                }
                .context("Full document text required")?
                .to_owned();
                let binding = self.documents.get(&uri).and_then(|d| d.binding.clone());
                let version = td["version"].as_i64().unwrap_or(0);
                self.documents.insert(
                    uri.clone(),
                    Document {
                        text: text.clone(),
                        version,
                        binding: binding.clone(),
                    },
                );
                if let Some((_, measure)) = binding {
                    self.db.update(&measure, text.clone());
                }
                let a = analysis::analyze(&text, &self.db.model);
                let mut diagnostics:Vec<_>=a.diagnostics.iter().map(|d|json!({"range":range(&text,d.start,d.end),"message":d.message,"severity":2,"source":"lightingdax (subset parser)"})).collect();
                for cycle in self.db.cycles() {
                    if self.documents[&uri]
                        .binding
                        .as_ref()
                        .is_some_and(|(_, m)| cycle.contains(m))
                    {
                        diagnostics.push(json!({"range":range(&text,0,0),"message":format!("Measure cycle: {}",cycle.join(" -> ")),"severity":1,"source":"lightingdax"}));
                    }
                }
                write(
                    output,
                    json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"version":version,"diagnostics":diagnostics}}),
                )?;
                Ok(Value::Null)
            }
            "textDocument/didClose" => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    if let Some(d) = self.documents.remove(uri) {
                        if let Some((table, measure)) = d.binding {
                            if let Some((_, original)) = self
                                .project
                                .as_ref()
                                .and_then(|p| p.model.qualified_measure(Some(&table), &measure))
                            {
                                self.db.update(&measure, original.expression.clone());
                            }
                        }
                    }
                    write(
                        output,
                        json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":[]}}),
                    )?;
                }
                Ok(Value::Null)
            }
            "textDocument/completion" => {
                let mut items = Vec::new();
                for (name, signature) in analysis::FUNCTIONS {
                    items.push(json!({"label":name,"kind":3,"detail":format!("{signature} • unverified locally")}));
                }
                for t in &self.db.model.tables {
                    let table = format!("'{}'", t.name.replace('\'', "''"));
                    items.push(json!({"label":table,"kind":7}));
                    for c in &t.columns {
                        items.push(json!({"label":format!("{table}[{}]",c.name.replace(']',"]]")),"kind":5,"detail":c.data_type}));
                    }
                    for m in &t.measures {
                        items.push(json!({"label":format!("[{}]",m.name.replace(']',"]]")),"kind":6,"detail":format!("Measure in {}",t.name)}));
                    }
                }
                Ok(json!({"isIncomplete":false,"items":items}))
            }
            "textDocument/formatting" => {
                let document = self.document(params)?;
                Ok(
                    json!([{"range":range(&document.text,0,document.text.len()),"newText":syntax::format(&document.text)}]),
                )
            }
            "textDocument/definition" => {
                let document = self.document(params)?;
                let byte = byte_position(&document.text, &params["position"]);
                let (tokens, _) = syntax::lex(&document.text);
                let name = tokens
                    .iter()
                    .find(|t| t.start <= byte && byte <= t.end)
                    .and_then(|t| {
                        if let syntax::Kind::Reference(n) = &t.kind {
                            Some(n)
                        } else {
                            None
                        }
                    });
                if let Some(source) = name
                    .and_then(|n| self.db.model.measure(n))
                    .and_then(|(_, m)| m.source.as_ref())
                {
                    let text = std::fs::read_to_string(&source.path)?;
                    return Ok(
                        json!({"uri":file_uri(&source.path),"range":range(&text,source.start,source.end)}),
                    );
                }
                Ok(Value::Null)
            }
            "lightingdax/measures" => Ok(json!(self
                .db
                .model
                .tables
                .iter()
                .flat_map(|t| t
                    .measures
                    .iter()
                    .map(|m| json!({"table":t.name,"name":m.name,"expression":m.expression})))
                .collect::<Vec<_>>())),
            "lightingdax/bindMeasure" => {
                let uri = params["uri"].as_str().context("Missing URI")?;
                let table = params["table"].as_str().context("Missing table")?;
                let name = params["name"].as_str().context("Missing measure")?;
                if self.db.model.qualified_measure(Some(table), name).is_none() {
                    bail!("Unknown measure");
                }
                let d = self
                    .documents
                    .get_mut(uri)
                    .context("Document has not opened")?;
                d.binding = Some((table.into(), name.into()));
                self.db.update(name, d.text.clone());
                Ok(Value::Null)
            }
            "lightingdax/saveMeasure" => {
                let uri = params["uri"].as_str().context("Missing URI")?;
                let d = self.documents.get(uri).context("Unknown document")?;
                let (table, name) = d
                    .binding
                    .as_ref()
                    .context("Open a model measure before saving")?;
                let project = self.project.as_ref().context("No project")?;
                project.write_measure(table, name, &d.text)?;
                let mut reload = Project::open(&project.root)?;
                if let Some(engine) = &mut self.engine {
                    reload.apply_authoritative_model(engine.load_model(&reload.definition)?);
                }
                self.project = Some(reload.clone());
                self.db = Database::new(reload.model);
                for d in self.documents.values() {
                    if let Some((_, name)) = &d.binding {
                        self.db.update(name, d.text.clone());
                    }
                }
                Ok(json!({"saved":true}))
            }
            "lightingdax/evaluate" => {
                let uri = params["uri"].as_str().context("Missing URI")?;
                let d = self.documents.get(uri).context("Unknown document")?;
                let text = d.text.clone();
                let version = d.version;
                let evaluation = if let Some(engine) = &mut self.engine {
                    let mut definitions = String::new();
                    for t in &self.db.model.tables {
                        for m in &t.measures {
                            if self
                                .project
                                .as_ref()
                                .and_then(|p| p.model.qualified_measure(Some(&t.name), &m.name))
                                .is_some_and(|(_, old)| old.expression != m.expression)
                            {
                                definitions.push_str(&format!(
                                    "MEASURE '{}'[{}] = {}\n",
                                    t.name.replace('\'', "''"),
                                    m.name.replace(']', "]]"),
                                    m.expression
                                ));
                            }
                        }
                    }
                    let body = if text
                        .trim_start()
                        .to_ascii_uppercase()
                        .starts_with("EVALUATE ")
                    {
                        text.clone()
                    } else {
                        format!("EVALUATE ROW(\"Value\", {text})")
                    };
                    if !definitions.is_empty()
                        && text
                            .trim_start()
                            .to_ascii_uppercase()
                            .starts_with("DEFINE ")
                    {
                        bail!("Query with DEFINE and unsaved measure overrides requires saving the overrides first");
                    }
                    let query = if definitions.is_empty() {
                        if text
                            .trim_start()
                            .to_ascii_uppercase()
                            .starts_with("DEFINE ")
                        {
                            text.clone()
                        } else {
                            body
                        }
                    } else {
                        format!("DEFINE\n{definitions}{body}")
                    };
                    Evaluation::engine(engine.run_query(&query)?)
                } else {
                    Evaluation::disconnected(&text, &self.db.model)
                };
                Ok(json!({"uri":uri,"version":version,"evaluation":evaluation}))
            }
            "lightingdax/visuals" => Ok(json!(self.project.as_ref().map(|p| &p.visuals))),
            _ => bail!("Method not supported: {method}"),
        }
    }
    fn document(&self, params: &Value) -> Result<&Document> {
        self.documents
            .get(
                params["textDocument"]["uri"]
                    .as_str()
                    .context("Missing URI")?,
            )
            .context("Unknown document")
    }
}
fn byte_position(text: &str, p: &Value) -> usize {
    let line = p["line"].as_u64().unwrap_or(0) as usize;
    let character = p["character"].as_u64().unwrap_or(0) as usize;
    let mut offset = 0;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i == line {
            let mut units = 0;
            for (byte, c) in l.char_indices() {
                if units >= character {
                    return offset + byte;
                }
                units += c.len_utf16();
            }
            return offset + l.len();
        }
        offset += l.len();
    }
    text.len()
}
fn file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::new();
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

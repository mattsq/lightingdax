use crate::{
    model::Model,
    snapshot::{read_arrow, restore_types, ResultTable},
};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

/// Sequential NDJSON protocol. stdout is exclusively protocol, stderr diagnostics.
pub struct Adapter {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: u64,
}
impl Adapter {
    pub fn start(program: &str, args: &[String]) -> Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Could not start engine adapter")?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child,
            input,
            output,
            id: 0,
        })
    }
    pub fn request(&mut self, command: &str, params: Value) -> Result<Value> {
        self.id += 1;
        let request = json!({"id":self.id,"command":command,"params":params});
        serde_json::to_writer(&mut self.input, &request)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let mut line = String::new();
        if self.output.read_line(&mut line)? == 0 {
            bail!("Engine adapter closed stdout");
        }
        let response: Value = serde_json::from_str(&line).context("Invalid adapter response")?;
        if response["id"] != self.id {
            bail!("Mismatched adapter request id");
        }
        if let Some(error) = response.get("error") {
            bail!(
                "Engine adapter: {}",
                error["message"].as_str().unwrap_or("unknown error")
            );
        }
        response
            .get("result")
            .cloned()
            .context("Missing adapter result")
    }
    pub fn load_model(&mut self, path: &Path) -> Result<Model> {
        serde_json::from_value(self.request("load_model", json!({"path":path}))?)
            .map_err(Into::into)
    }
    pub fn run_query(&mut self, query: &str) -> Result<ResultTable> {
        let data = self.request("run_query", json!({"query":query}))?;
        let path = data["path"].as_str().context("Missing Arrow result path")?;
        let result = (|| {
            let mut table = read_arrow(Path::new(path))?;
            let types: Vec<String> = serde_json::from_value(data["types"].clone())?;
            restore_types(&mut table, &types)?;
            Ok(table)
        })();
        // Only ephemeral query results are deleted; snapshots are caller-owned.
        if data["temporary"].as_bool() == Some(true) {
            let _ = std::fs::remove_file(path);
        }
        result
    }
}
impl Drop for Adapter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

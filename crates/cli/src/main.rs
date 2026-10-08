mod server;
use anyhow::{bail, Context, Result};
use lightingdax_core::{
    adapter::Adapter,
    analysis,
    model::Model,
    project::Project,
    snapshot::load_manifest,
    trust::{self, Corpus, Evaluation, OracleCase},
};
use serde_json::json;
use std::{env, fs, path::Path};

fn adapter() -> Result<Option<Adapter>> {
    match env::var("LIGHTINGDAX_ADAPTER") {
        Ok(program) => {
            let args: Vec<String> = serde_json::from_str(
                &env::var("LIGHTINGDAX_ADAPTER_ARGS").unwrap_or_else(|_| "[]".into()),
            )?;
            Ok(Some(Adapter::start(&program, &args)?))
        }
        Err(_) => Ok(None),
    }
}
fn open(path: &str, engine: &mut Option<Adapter>) -> Result<Project> {
    let mut project = Project::open(Path::new(path))?;
    if let Some(engine) = engine {
        let model = engine.load_model(&project.definition)?;
        project.apply_authoritative_model(model);
    }
    Ok(project)
}
fn print(value: impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("lightingdax: {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");
    if command == "lsp" {
        return server::run();
    }
    if command == "help" || command == "--help" {
        println!("lightingdax inspect PROJECT\nlightingdax check PROJECT\nlightingdax eval PROJECT EXPRESSION\nlightingdax query PROJECT QUERY\nlightingdax save PROJECT TABLE MEASURE EXPRESSION\nlightingdax snapshot PROJECT TABLE OUTPUT.arrow\nlightingdax export-model PROJECT OUTPUT.json\nlightingdax fingerprint MODEL.json SNAPSHOTS.json\nlightingdax record PROJECT SNAPSHOTS.json CASES.json CORPUS.json\nlightingdax test MODEL.json SNAPSHOTS.json CORPUS.json\nlightingdax lsp\n\nEngine adapter: LIGHTINGDAX_ADAPTER executable and LIGHTINGDAX_ADAPTER_ARGS JSON array.\nConnection string: LIGHTINGDAX_CONNECTION (inherited by the adapter).\nNo functions currently have verified local coverage.");
        return Ok(());
    }
    let path = args.get(1).context("Expected project path (see --help)")?;
    // Corpus replay is independent of Windows, even if an adapter is configured.
    if command == "test" || command == "fingerprint" {
        let model: Model = if path.ends_with(".json") {
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            Project::open(Path::new(path))?.model
        };
        let manifest = args.get(2).context("Expected snapshot manifest")?;
        let store = load_manifest(Path::new(manifest))?;
        if command == "fingerprint" {
            return print(json!({"fingerprint":trust::fingerprint(&model,&store)?}));
        }
        let corpus: Corpus =
            serde_json::from_slice(&fs::read(args.get(3).context("Expected corpus path")?)?)?;
        let passed = trust::replay(&corpus, &model, &store)?;
        return print(json!({"passed":passed,"fingerprint":trust::fingerprint(&model,&store)?}));
    }
    let mut engine = adapter()?;
    let project = open(path, &mut engine)?;
    match command {
        "inspect" => print(&project),
        "check" => {
            let mut db = analysis::Database::new(project.model);
            let names: Vec<_> = db
                .model
                .tables
                .iter()
                .flat_map(|t| t.measures.iter().map(|m| m.name.clone()))
                .collect();
            let mut diagnostics = Vec::new();
            for name in names {
                if let Some(a) = db.analysis(&name) {
                    for d in &a.diagnostics {
                        diagnostics.push(json!({"measure":name,"diagnostic":d}));
                    }
                }
            }
            let cycles = db.cycles();
            print(json!({"diagnostics":diagnostics,"cycles":cycles}))?;
            if !diagnostics.is_empty() || !cycles.is_empty() {
                bail!("Measure diagnostics found (the scalar parser has limited coverage)");
            }
            Ok(())
        }
        "eval" | "query" => {
            let text = args.get(2).context("Expected expression or query")?;
            let evaluation = if let Some(engine) = &mut engine {
                let query = if command == "eval" {
                    format!("EVALUATE ROW(\"Value\", {text})")
                } else {
                    text.clone()
                };
                Evaluation::engine(engine.run_query(&query)?)
            } else {
                Evaluation::disconnected(text, &project.model)
            };
            print(evaluation)
        }
        "save" => {
            if args.len() != 5 {
                bail!("save requires PROJECT TABLE MEASURE EXPRESSION");
            }
            project.write_measure(&args[2], &args[3], &args[4])?;
            print(json!({"saved":true}))
        }
        "snapshot" => {
            if args.len() != 4 {
                bail!("snapshot requires PROJECT TABLE OUTPUT.arrow");
            }
            let output = Path::new(&args[3]);
            let output = if output.is_absolute() {
                output.to_path_buf()
            } else {
                env::current_dir()?.join(output)
            };
            let result = engine
                .as_mut()
                .context("Snapshots require an engine adapter")?
                .request("snapshot_table", json!({"table":args[2],"path":output}))?;
            print(result)
        }
        "export-model" => {
            let destination = args.get(2).context("Expected new model JSON path")?;
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)?;
            file.write_all(&serde_json::to_vec_pretty(&project.model)?)?;
            print(json!({"saved":destination}))
        }
        "record" => {
            if args.len() != 5 {
                bail!("record requires PROJECT SNAPSHOTS.json CASES.json CORPUS.json");
            }
            let engine = engine
                .as_mut()
                .context("Recording requires a real engine adapter")?;
            let store = load_manifest(Path::new(&args[2]))?;
            let fingerprint = trust::fingerprint(&project.model, &store)?;
            let cases: std::collections::BTreeMap<String, String> =
                serde_json::from_slice(&fs::read(&args[3])?)?;
            let version = engine.request("engine_info", json!({}))?["version"]
                .as_str()
                .context("Engine did not return its version")?
                .to_owned();
            if cases.is_empty() {
                bail!("No cases to record");
            }
            for (table, snapshot) in &store.tables {
                let live =
                    engine.run_query(&format!("EVALUATE '{}'", table.replace('\'', "''")))?;
                lightingdax_core::snapshot::compare(&live, snapshot, 0.0).with_context(|| {
                    format!(
                        "Snapshot for {table} differs from the engine. Re-export before recording"
                    )
                })?;
            }
            let mut recorded = Vec::new();
            for (name, expression) in cases {
                let mut expected =
                    engine.run_query(&format!("EVALUATE ROW(\"Value\", {expression})"))?;
                if expected.columns.len() != 1 || expected.rows.len() != 1 {
                    bail!("Scalar oracle query returned a non-scalar result");
                }
                expected.columns = vec!["Value".into()];
                recorded.push(OracleCase {
                    name,
                    expression,
                    engine_version: version.clone(),
                    fingerprint: fingerprint.clone(),
                    expected,
                });
            }
            // Snapshots were checked above. Freeze the model while recording: engine
            // metadata/data can still change between requests (see docs/oracle.md).
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[4])?;
            file.write_all(&serde_json::to_vec_pretty(&Corpus { cases: recorded })?)?;
            print(json!({"recorded":args[4],"fingerprint":fingerprint}))
        }
        _ => bail!("Unknown command '{command}' (see --help)"),
    }
}

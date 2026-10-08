use lightingdax_core::{
    analysis::{self, Database},
    eval::{FilterContext, Interpreter},
    model::*,
    project::{read_table, Project},
    snapshot::{self, ResultTable, Store, Value},
    syntax::{self, Expr, Kind},
    trust::{self, Evaluation, Status},
};
use std::{collections::BTreeMap, fs, path::PathBuf};
#[test]
fn integer_wire_format_preserves_every_i64_digit() {
    let value = Value::Integer(9007199254740993);
    let json = serde_json::to_string(&value).unwrap();
    assert!(json.contains("\"9007199254740993\""));
    assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), value);
}
#[test]
fn adomd_qualified_snapshot_columns_resolve() {
    let m = model();
    let mut s = store();
    s.tables.get_mut("Sales").unwrap().columns = vec!["Sales[Amount]".into()];
    assert_eq!(
        Interpreter::new(&m, &s).evaluate("[Revenue]").unwrap(),
        Value::Number(50.)
    );
}
#[test]
fn multiline_string_save_refuses_to_change_string_contents() {
    let p = temp();
    let file = p.join("tables/T.tmdl");
    let original = "table T\n\tmeasure X = 1\n";
    fs::write(&file, original).unwrap();
    let project = Project::open(&p).unwrap();
    assert!(project
        .write_measure("T", "X", "\"first\n  second\"")
        .is_err());
    assert_eq!(fs::read_to_string(file).unwrap(), original);
    fs::remove_dir_all(p).unwrap();
}
fn model() -> Model {
    Model {
        tables: vec![Table {
            name: "Sales".into(),
            columns: vec![Column {
                name: "Amount".into(),
                data_type: "double".into(),
            }],
            measures: vec![
                Measure {
                    name: "Revenue".into(),
                    expression: "SUM(Sales[Amount])".into(),
                    source: None,
                },
                Measure {
                    name: "Double".into(),
                    expression: "[Revenue] * 2".into(),
                    source: None,
                },
                Measure {
                    name: "Constant".into(),
                    expression: "1".into(),
                    source: None,
                },
            ],
        }],
        relationships: vec![],
    }
}
fn store() -> Store {
    Store {
        tables: BTreeMap::from([(
            "Sales".into(),
            ResultTable {
                columns: vec!["Amount".into()],
                rows: vec![
                    vec![Value::Number(20.)],
                    vec![Value::Number(30.)],
                    vec![Value::Blank],
                ],
            },
        )]),
    }
}
fn temp() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ID: AtomicUsize = AtomicUsize::new(0);
    let p = std::env::temp_dir().join(format!(
        "lightingdax-test-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(p.join("tables")).unwrap();
    p
}

#[test]
fn lossless_unicode_and_comments() {
    let text = "SUM('Café''s'[A]]B]) /* test */ + \"a\"\"b\"";
    let (tokens, d) = syntax::lex(text);
    assert!(d.is_empty());
    let joined: String = tokens
        .iter()
        .filter(|t| t.kind != Kind::End)
        .map(|t| &text[t.start..t.end])
        .collect();
    assert_eq!(joined, text);
    assert!(syntax::parse(text).diagnostics.is_empty());
}
#[test]
fn incomplete_expression_recovers() {
    let p = syntax::parse("SUM(Sales[");
    assert!(!p.diagnostics.is_empty());
    assert!(matches!(p.expression, Expr::Call(_, _)));
}
#[test]
fn malformed_and_excessively_nested_inputs_do_not_panic() {
    for text in ["", "(", "SUM(,)", "1e+", "/*", "[a", "☃"] {
        syntax::parse(text);
    }
    let text = "(".repeat(300);
    assert!(!syntax::parse(&text).diagnostics.is_empty());
}
#[test]
fn precedence() {
    let m = model();
    let s = store();
    let mut i = Interpreter::new(&m, &s);
    assert_eq!(i.evaluate("2+3*4").unwrap(), Value::Number(14.));
}
#[test]
fn blank_rules() {
    let m = model();
    let s = store();
    let mut i = Interpreter::new(&m, &s);
    assert_eq!(i.evaluate("BLANK()+5").unwrap(), Value::Number(5.));
    assert_eq!(i.evaluate("BLANK()=0").unwrap(), Value::Boolean(true));
    assert_eq!(i.evaluate("BLANK()==0").unwrap(), Value::Boolean(false));
    assert_eq!(i.evaluate("BLANK()*5").unwrap(), Value::Blank);
}
#[test]
fn divide_blank_and_lazy_if() {
    let m = model();
    let s = store();
    let mut i = Interpreter::new(&m, &s);
    assert_eq!(i.evaluate("DIVIDE(1,0)").unwrap(), Value::Blank);
    assert_eq!(i.evaluate("DIVIDE(BLANK(),2)").unwrap(), Value::Blank);
    assert_eq!(
        i.evaluate("IF(TRUE(), 3, DOESNOTEXIST())").unwrap(),
        Value::Number(3.)
    );
}
#[test]
fn aggregates_ignore_blank() {
    let m = model();
    let s = store();
    let mut i = Interpreter::new(&m, &s);
    assert_eq!(i.evaluate("[Revenue]").unwrap(), Value::Number(50.));
    assert_eq!(
        i.evaluate("AVERAGE(Sales[Amount])").unwrap(),
        Value::Number(25.)
    );
    assert_eq!(
        i.evaluate("COUNT(Sales[Amount])").unwrap(),
        Value::Integer(2)
    );
}
#[test]
fn implicit_measure_context_transition() {
    let m = model();
    let s = store();
    let mut i = Interpreter::new(&m, &s);
    assert_eq!(
        i.evaluate("SUMX(Sales, [Revenue])").unwrap(),
        Value::Number(50.)
    );
    assert_eq!(
        i.evaluate("SUMX(Sales, SUM(Sales[Amount]))").unwrap(),
        Value::Number(150.)
    );
    assert!(i.row.rows.is_empty());
    assert!(i.filters.columns.is_empty());
}
#[test]
fn row_context_is_restored_on_error() {
    let m = model();
    let s = store();
    let mut i = Interpreter::new(&m, &s);
    assert!(i.evaluate("SUMX(Sales, UNSUPPORTED())").is_err());
    assert!(i.row.rows.is_empty());
}
#[test]
fn filters_are_per_column() {
    let m = model();
    let s = store();
    let mut i = Interpreter::new(&m, &s);
    i.filters = FilterContext {
        columns: BTreeMap::from([(("sales".into(), "amount".into()), vec![Value::Number(20.)])]),
    };
    assert_eq!(i.evaluate("[Revenue]").unwrap(), Value::Number(20.));
}
#[test]
fn unknown_relationship_semantics_are_rejected() {
    let mut m = model();
    m.relationships.push(Relationship {
        from_table: "Sales".into(),
        from_column: "Amount".into(),
        to_table: "D".into(),
        to_column: "K".into(),
        active: true,
        single_direction: true,
    });
    assert!(Interpreter::new(&m, &store())
        .evaluate("[Revenue]")
        .is_err());
}
#[test]
fn incremental_reverse_dependencies() {
    let mut db = Database::new(model());
    db.analysis("Constant");
    let affected = db.update("Revenue", "1".into()).unwrap();
    assert_eq!(
        affected.into_iter().collect::<Vec<_>>(),
        vec!["Double", "Revenue"]
    );
    assert_eq!(db.revision, 1);
}
#[test]
fn cycle_diagnostics_and_evaluation() {
    let mut db = Database::new(model());
    db.update("Revenue", "[Double]".into());
    assert!(!db.cycles().is_empty());
    assert!(Interpreter::new(&db.model, &store())
        .evaluate("[Revenue]")
        .is_err());
}
#[test]
fn binder_resolves_case_insensitively() {
    let a = analysis::analyze("sum(sales[amount]) + [revenue]", &model());
    assert!(a.diagnostics.is_empty());
    assert!(a.dependencies.contains("Revenue"));
    assert!(!analysis::analyze("Sales[Missing]", &model())
        .diagnostics
        .is_empty());
}
#[test]
fn no_unverified_numbers() {
    let e = Evaluation::disconnected("[Double]", &model());
    assert_eq!(e.status, Status::Unverified);
    assert!(e.result.is_none());
    assert!(e.warning.is_some());
    assert_eq!(
        trust::coverage("CALCULATE([Revenue])", &model()),
        Status::Unsupported
    );
}
#[test]
fn unsupported_dependency_propagates() {
    let mut db = Database::new(model());
    db.update("Revenue", "CALCULATE(1)".into());
    assert_eq!(trust::coverage("[Double]", &db.model), Status::Unsupported);
}
#[test]
fn compare_requires_shape_types_and_blanks() {
    let table = trust::scalar(Value::Blank);
    assert!(snapshot::compare(&table, &trust::scalar(Value::Number(0.)), 1e-9).is_err());
    assert!(snapshot::compare(
        &trust::scalar(Value::Integer(1)),
        &trust::scalar(Value::Number(1.)),
        1e-9
    )
    .is_err());
    assert!(snapshot::compare(
        &trust::scalar(Value::Number(1.)),
        &trust::scalar(Value::Number(1. + 1e-10)),
        1e-9
    )
    .is_ok());
    assert!(snapshot::compare(&table, &table, -1.).is_err());
}
#[test]
fn fingerprint_ignores_source_but_detects_data_and_model_changes() {
    let m = model();
    let s = store();
    let original = trust::fingerprint(&m, &s).unwrap();
    let mut changed = m.clone();
    changed.tables[0].measures[0].source = Some(Source {
        path: "a".into(),
        start: 1,
        end: 2,
        revision: "ignored".into(),
    });
    assert_eq!(original, trust::fingerprint(&changed, &s).unwrap());
    changed.tables[0].measures[0].expression = "1".into();
    assert_ne!(original, trust::fingerprint(&changed, &s).unwrap());
    let mut data = s.clone();
    data.tables.get_mut("Sales").unwrap().rows.clear();
    assert_ne!(original, trust::fingerprint(&m, &data).unwrap());
}
#[test]
fn corpus_rejects_stale_or_empty_evidence() {
    let m = model();
    let s = store();
    assert!(trust::replay(&trust::Corpus { cases: vec![] }, &m, &s).is_err());
    let corpus = trust::Corpus {
        cases: vec![trust::OracleCase {
            name: "synthetic unit test, not engine evidence".into(),
            expression: "[Revenue]".into(),
            engine_version: "test-double".into(),
            fingerprint: "stale".into(),
            expected: trust::scalar(Value::Number(50.)),
        }],
    };
    assert!(trust::replay(&corpus, &m, &s).is_err());
}
#[test]
fn targeted_save_preserves_metadata_and_crlf() {
    let p = temp();
    let file = p.join("tables/Sales.tmdl");
    let text="table Sales\r\n\tmeasure 'A=B' = 1 + 2\r\n\t\tformatString: 0.00\r\n\t\tlineageTag: unchanged\r\n\tmeasure B =\r\n\t\t3 +\r\n\t\t4\r\n\t\tformatString: 0\r\n";
    fs::write(&file, text).unwrap();
    let project = Project::open(&p).unwrap();
    assert_eq!(project.model.tables[0].measures[0].name, "A=B");
    project
        .write_measure("Sales", "A=B", "SUM(Sales[Amount])")
        .unwrap();
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        text.replace("1 + 2", "SUM(Sales[Amount])")
    );
    assert!(project.write_measure("Sales", "B", "5").is_err());
    let new = Project::open(&p).unwrap();
    new.write_measure("Sales", "B", "5 +\n6").unwrap();
    let after = fs::read_to_string(&file).unwrap();
    assert!(after.contains("\r\n\t\t5 +\r\n\t\t6\r\n\t\tformatString: 0"));
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn fenced_expressions_are_rejected_without_safe_mapping() {
    let p = temp();
    let file = p.join("tables/T.tmdl");
    fs::write(&file, "table T\n\tmeasure X = ```\n\t\t1\n\t\t```\n").unwrap();
    assert!(read_table(&file).is_err());
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn formatter_preserves_strings_and_comments() {
    assert_eq!(
        syntax::format("sum('T'[A]) // sum()\n + \"sum(\""),
        "SUM('T'[A]) // sum()\n + \"sum(\""
    );
}
#[test]
fn arrow_roundtrip_preserves_nullable_types() {
    use arrow::{
        array::{ArrayRef, Float64Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        ipc::writer::FileWriter,
        record_batch::RecordBatch,
    };
    use std::sync::Arc;
    let p = temp();
    let path = p.join("test.arrow");
    let schema = Arc::new(Schema::new(vec![
        Field::new("Amount", DataType::Float64, true),
        Field::new("Count", DataType::Int64, true),
        Field::new("Label", DataType::Utf8, true),
    ]));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Float64Array::from(vec![Some(2.5), None])),
        Arc::new(Int64Array::from(vec![Some(9007199254740993), None])),
        Arc::new(StringArray::from(vec![Some("<script>"), None])),
    ];
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    let mut writer = FileWriter::try_new(fs::File::create(&path).unwrap(), &schema).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    let result = snapshot::read_arrow(&path).unwrap();
    assert_eq!(result.rows[0][1], Value::Integer(9007199254740993));
    assert_eq!(result.rows[1], vec![Value::Blank; 3]);
    fs::remove_dir_all(p).unwrap();
}

use super::*;

fn setup(read_only: bool) -> (tempfile::TempDir, Armarium) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(
        data.join("sales.csv"),
        "部署,金額\n営業,100\n開発,200\n営業,50\n",
    )
    .unwrap();
    let cfg = armarium_core::EngineConfig {
        workspace_path: dir.path().join("workspace.json"),
        temp_dir: dir.path().join("tmp"),
        ..Default::default()
    };
    let engine = Arc::new(Engine::new(cfg, Arc::new(armarium_core::NoEvents)).unwrap());
    let opts = Options {
        read_only,
        allow_dirs: vec![std::fs::canonicalize(&data).unwrap()],
        timeout: Duration::from_secs(10),
        max_rows: 2,
    };
    (dir, Armarium::new(engine, opts))
}

fn structured(r: &CallToolResult) -> &Value {
    r.structured_content.as_ref().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_work_end_to_end() {
    let (dir, s) = setup(false);
    let path = std::fs::canonicalize(dir.path().join("data/sales.csv")).unwrap();
    let r = s
        .register_source(Parameters(RegisterArgs {
            path: path.display().to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(false), "{:?}", r.content);

    let r = s.list_sources().await.unwrap();
    assert_eq!(
        structured(&r)["sources"][0]["tables"][0]["sqlName"],
        "sales"
    );

    let r = s
        .describe_table(Parameters(TableArgs {
            table: "sales".into(),
        }))
        .await
        .unwrap();
    assert_eq!(structured(&r)["row_count"], 3);
    assert_eq!(structured(&r)["columns"][1]["type"], "integer");

    let r = s
        .query(Parameters(QueryArgs {
            sql: "SELECT 部署, SUM(金額) AS 合計 FROM sales GROUP BY 部署 ORDER BY 合計 DESC"
                .into(),
            max_rows: Some(100),
        }))
        .await
        .unwrap();
    let v = structured(&r);
    assert_eq!(v["rows"], json!([["開発", 200], ["営業", 150]]));
    assert_eq!(v["truncated"], false);

    // --max-rows limits the rows.
    let r = s
        .query(Parameters(QueryArgs {
            sql: "SELECT * FROM sales".into(),
            max_rows: None,
        }))
        .await
        .unwrap();
    assert_eq!(structured(&r)["truncated"], true);
    assert_eq!(structured(&r)["row_count"], 3);

    // query refuses DML.
    let r = s
        .query(Parameters(QueryArgs {
            sql: "DELETE FROM sales".into(),
            max_rows: None,
        }))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(true));

    let r = s
        .execute(Parameters(ExecuteArgs {
            sql: "UPDATE sales SET 金額 = 金額 + 1 WHERE 部署 = '営業'".into(),
        }))
        .await
        .unwrap();
    assert_eq!(structured(&r)["results"][0]["changed_rows"], 2);

    let r = s
        .diff_table(Parameters(DiffArgs {
            table: "sales".into(),
            sample: None,
        }))
        .await
        .unwrap();
    assert_eq!(structured(&r)["changedRows"], 2);

    let r = s
        .save_table(Parameters(TableArgs {
            table: "sales".into(),
        }))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(false), "{:?}", r.content);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "部署,金額\n営業,101\n開発,200\n営業,51\n"
    );

    let out = std::fs::canonicalize(dir.path().join("data"))
        .unwrap()
        .join("out.csv");
    let r = s
        .export_result(Parameters(ExportArgs {
            sql: "SELECT * FROM sales".into(),
            path: out.display().to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(false), "{:?}", r.content);
    assert!(out.exists());

    // Paths outside --allow-dir are refused.
    let r = s
        .export_result(Parameters(ExportArgs {
            sql: "SELECT 1".into(),
            path: dir.path().join("outside.csv").display().to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(true));

    let stats = Armarium::column_stats(&s.engine, "sales").unwrap();
    assert!(stats.contains("distinct_values"), "{stats}");

    let r = s
        .unregister_source(Parameters(UnregisterArgs {
            source: "sales".into(),
        }))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(false));
    assert!(path.exists());
}

#[test]
fn read_only_mode_removes_write_tools() {
    let (_dir, s) = setup(true);
    let names: Vec<String> = s
        .tool_router
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for w in WRITE_TOOLS {
        assert!(!names.contains(&w.to_string()));
    }
    assert!(names.contains(&"query".to_string()));
}

#[test]
fn parses_resource_uris() {
    let enc = percent_encode("売上 2026");
    let uri = format!("armarium://tables/{enc}/sample");
    let (name, kind) = parse_uri(&uri).unwrap();
    assert_eq!((name.as_str(), kind), ("売上 2026", "sample"));
}

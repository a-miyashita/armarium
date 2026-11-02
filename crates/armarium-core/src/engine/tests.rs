use super::*;
use crate::sql::SqlOptions;

struct Env {
    dir: tempfile::TempDir,
    engine: Engine,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let cfg = EngineConfig {
        workspace_path: dir.path().join("cfg").join("workspace.json"),
        temp_dir: dir.path().join("tmp"),
        scratch_root: dir.path().join("scratch"),
        scratch_idle: Duration::ZERO,
        ..EngineConfig::default()
    };
    let engine = Engine::new(cfg, Arc::new(NoEvents)).unwrap();
    Env { dir, engine }
}

impl Env {
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn add(&self, path: &Path) -> TableId {
        let r = self.engine.add_sources(&[path.to_path_buf()]).unwrap();
        let sid = r[0]
            .source_id
            .clone()
            .unwrap_or_else(|| panic!("{:?}", r[0].error));
        let src = self
            .engine
            .list_sources()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap();
        src.tables[0].id
    }

    fn sql(&self, text: &str) -> crate::sql::SqlRun {
        self.engine.run_sql(text, &SqlOptions::default())
    }

    fn rows(&self, t: TableId) -> Vec<Vec<Option<String>>> {
        let p = self.engine.fetch_rows(t, None, 0, 1000).unwrap();
        (0..p.rids.len())
            .map(|r| p.data.iter().map(|c| c[r].clone()).collect())
            .collect()
    }
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

const SAMPLES: &[(&str, &[u8])] = &[
    (
        "plain.csv",
        b"id,name,amount\r\n1,Apple,120\r\n2,Banana,80\r\n",
    ),
    ("lf_noeol.csv", b"id,name\n1,a\n2,b"),
    (
        "quoted.csv",
        b"\"id\",\"name\"\n\"1\",\"a, b\"\n\"2\",\"say \"\"hi\"\"\"\n",
    ),
    (
        "mixed_quote.csv",
        b"id,name,note\n1,\"Taro\",x\n2,\"Hanako\",\"y, z\"\n",
    ),
    (
        "types.csv",
        b"zip,price,d,r\n00123,1.50,2026-01-02,0.5\n00456,2.00,2026/01/03,1.25\n",
    ),
    ("semi.csv", b"a;b\r\n1;2\r\n;3\r\n"),
    ("tabs.tsv", b"x\ty\n1\t\n\t2\n"),
    ("bom.csv", b"\xEF\xBB\xBFa,b\r\n1,2\r\n"),
    ("noheader.csv", b"1,2,3\n4,5,6\n"),
    (
        "multiline.csv",
        b"id,text\r\n1,\"line1\r\nline2\"\r\n2,x\r\n",
    ),
    ("cr.csv", b"a,b\r1,2\r3,4\r"),
    (
        "datetime.csv",
        b"ts,v\n2026-01-02 03:04:05,1\n2026-01-03T10:00:00,2\n",
    ),
];

#[test]
fn open_and_save_is_byte_for_byte_identical() {
    // M2-7: "open → save" gives the same bytes for all test data.
    let e = env();
    let mut all: Vec<(&str, Vec<u8>)> = SAMPLES.iter().map(|(n, b)| (*n, b.to_vec())).collect();
    let (sjis, _, _) =
        encoding_rs::SHIFT_JIS.encode("部署,金額\r\n営業,1200000\r\n開発,800000\r\n");
    all.push(("sjis.csv", sjis.into_owned()));
    let mut u16 = vec![0xFF, 0xFE];
    for u in "a\tb\r\n1\t2\r\n".encode_utf16() {
        u16.extend_from_slice(&u.to_le_bytes());
    }
    all.push(("utf16.txt", u16));
    let (euc, _, _) = encoding_rs::EUC_JP.encode("名前,値\n日本,1\n");
    all.push(("euc.csv", euc.into_owned()));
    for (name, bytes) in all {
        let p = e.file(name, &bytes);
        let t = e.add(&p);
        e.engine.open_table(t).unwrap();
        // Make the table dirty and clean again, then save with force.
        e.engine.save_table(t, false).unwrap();
        let after = std::fs::read(&p).unwrap();
        assert_eq!(
            after,
            bytes,
            "round trip of {name}: {:?}",
            String::from_utf8_lossy(&after)
        );
    }
}

#[test]
fn edits_undo_redo_and_dirty_state() {
    let e = env();
    let p = e.file(
        "sales.csv",
        b"id,name,amount\r\n1,Apple,120\r\n2,Banana,80\r\n",
    );
    let t = e.add(&p);
    let meta = e.engine.open_table(t).unwrap();
    assert_eq!(meta.columns[2].logical_type, LogicalType::Integer);
    let page = e.engine.fetch_rows(t, None, 0, 10).unwrap();
    let rid2 = page.rids[1];

    let r = e
        .engine
        .apply_edit(
            t,
            EditOp::SetCells {
                cells: vec![CellEdit {
                    rid: rid2,
                    column: "amount".into(),
                    value: s("1,000"),
                }],
            },
        )
        .unwrap();
    assert!(r.table.dirty);
    assert_eq!(e.rows(t)[1][2], s("1000"));

    // Type mismatch changes nothing.
    let err = e
        .engine
        .apply_edit(
            t,
            EditOp::SetCells {
                cells: vec![CellEdit {
                    rid: rid2,
                    column: "amount".into(),
                    value: s("abc"),
                }],
            },
        )
        .unwrap_err();
    assert_eq!(err.code(), "TYPE_MISMATCH");

    // Insert a row between the two rows, then delete the first row.
    let r = e
        .engine
        .apply_edit(
            t,
            EditOp::InsertRows {
                after_rid: Some(page.rids[0]),
                count: 1,
                above: None,
            },
        )
        .unwrap();
    let new_rid = r.inserted_rids[0];
    e.engine
        .apply_edit(
            t,
            EditOp::SetCells {
                cells: vec![CellEdit {
                    rid: new_rid,
                    column: "name".into(),
                    value: s("Cherry"),
                }],
            },
        )
        .unwrap();
    assert_eq!(
        e.rows(t).iter().map(|r| r[1].clone()).collect::<Vec<_>>(),
        vec![s("Apple"), s("Cherry"), s("Banana")]
    );
    e.engine
        .apply_edit(
            t,
            EditOp::DeleteRows {
                rids: vec![page.rids[0]],
            },
        )
        .unwrap();
    assert_eq!(e.rows(t).len(), 2);

    // Undo everything: the table is not dirty again.
    for _ in 0..4 {
        e.engine.undo(t).unwrap();
    }
    let m = e.engine.table_meta(t).unwrap();
    assert!(!m.dirty);
    assert!(!m.can_undo);
    assert_eq!(e.rows(t)[0], vec![s("1"), s("Apple"), s("120")]);
    assert_eq!(e.rows(t).len(), 2);
    // Redo all.
    for _ in 0..4 {
        e.engine.redo(t).unwrap();
    }
    assert_eq!(
        e.rows(t),
        vec![
            vec![None, s("Cherry"), None],
            vec![s("2"), s("Banana"), s("1000")]
        ]
    );

    e.engine.save_table(t, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "id,name,amount\r\n,Cherry,\r\n2,Banana,1000\r\n"
    );
    assert!(!e.engine.table_meta(t).unwrap().dirty);
    // Undo after a save makes the table dirty again.
    e.engine.undo(t).unwrap();
    assert!(e.engine.table_meta(t).unwrap().dirty);
}

#[test]
fn insert_at_start_and_paste() {
    let e = env();
    let p = e.file("a.csv", b"a,b\n1,x\n2,y\n");
    let t = e.add(&p);
    e.engine.open_table(t).unwrap();
    e.engine
        .apply_edit(
            t,
            EditOp::InsertRows {
                after_rid: None,
                count: 2,
                above: None,
            },
        )
        .unwrap();
    let page = e.engine.fetch_rows(t, None, 0, 10).unwrap();
    assert_eq!(page.total, 4);
    assert_eq!(page.data[0][2], s("1"));
    // Insert above the last row.
    let r = e
        .engine
        .apply_edit(
            t,
            EditOp::InsertRows {
                after_rid: Some(page.rids[3]),
                count: 1,
                above: Some(true),
            },
        )
        .unwrap();
    let p2 = e.engine.fetch_rows(t, None, 0, 10).unwrap();
    assert_eq!(p2.rids[3], r.inserted_rids[0]);
    e.engine.undo(t).unwrap();
    // Paste 3 rows from the second row: one new row at the end.
    e.engine
        .apply_edit(
            t,
            EditOp::Paste {
                anchor_rid: page.rids[1],
                anchor_column: "a".into(),
                tsv: "10\tp\r\n11\t\"q\nr\"\r\n12\ts\r\n13\tt\r\n".into(),
                rids: page.rids[1..].to_vec(),
            },
        )
        .unwrap();
    let rows = e.rows(t);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[1], vec![s("10"), s("p")]);
    assert_eq!(rows[2], vec![s("11"), s("q\nr")]);
    assert_eq!(rows[4], vec![s("13"), s("t")]);
    e.engine.undo(t).unwrap();
    assert_eq!(e.rows(t).len(), 4);
}

#[test]
fn column_operations_and_undo() {
    let e = env();
    let p = e.file("c.csv", b"id,name,zip\n1,a,00123\n2,b,00456\n");
    let t = e.add(&p);
    e.engine.open_table(t).unwrap();
    e.engine
        .apply_edit(
            t,
            EditOp::InsertColumn {
                name: "status".into(),
                logical_type: LogicalType::Text,
                position: Some(1),
            },
        )
        .unwrap();
    let m = e.engine.table_meta(t).unwrap();
    assert_eq!(
        m.columns
            .iter()
            .map(|c| c.sql_name.as_str())
            .collect::<Vec<_>>(),
        vec!["id", "status", "name", "zip"]
    );
    e.engine
        .apply_edit(
            t,
            EditOp::DeleteColumn {
                column: "name".into(),
            },
        )
        .unwrap();
    e.engine
        .apply_edit(
            t,
            EditOp::RenameColumn {
                column: "zip".into(),
                to: "postal code".into(),
            },
        )
        .unwrap();
    let err = e
        .engine
        .apply_edit(
            t,
            EditOp::ChangeColumnType {
                column: "postal code".into(),
                logical_type: LogicalType::Date,
            },
        )
        .unwrap_err();
    assert_eq!(err.code(), "TYPE_MISMATCH");
    e.engine
        .apply_edit(
            t,
            EditOp::ChangeColumnType {
                column: "id".into(),
                logical_type: LogicalType::Text,
            },
        )
        .unwrap();
    let run = e.sql("UPDATE c SET status = 'ok' WHERE id = '2'");
    assert!(run.error.is_none(), "{:?}", run.error);
    e.engine.save_table(t, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "id,status,postal code\n1,,00123\n2,ok,00456\n"
    );
    // Undo all operations, in reverse order.
    for _ in 0..5 {
        e.engine.undo(t).unwrap();
    }
    let m = e.engine.table_meta(t).unwrap();
    assert_eq!(
        m.columns
            .iter()
            .map(|c| c.sql_name.as_str())
            .collect::<Vec<_>>(),
        vec!["id", "name", "zip"]
    );
    assert_eq!(m.columns[0].logical_type, LogicalType::Integer);
    assert_eq!(
        e.rows(t),
        vec![
            vec![s("1"), s("a"), s("00123")],
            vec![s("2"), s("b"), s("00456")]
        ]
    );
    e.engine.save_table(t, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "id,name,zip\n1,a,00123\n2,b,00456\n"
    );
}

#[test]
fn detects_external_modification() {
    let e = env();
    let p = e.file("x.csv", b"a\n1\n");
    let t = e.add(&p);
    e.engine.open_table(t).unwrap();
    let rid = e.engine.fetch_rows(t, None, 0, 1).unwrap().rids[0];
    e.engine
        .apply_edit(
            t,
            EditOp::SetCells {
                cells: vec![CellEdit {
                    rid,
                    column: "a".into(),
                    value: s("2"),
                }],
            },
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&p, "a\n9\n").unwrap();
    let err = e.engine.save_table(t, false).unwrap_err();
    assert_eq!(err.code(), "EXTERNAL_MODIFICATION");
    let changed = e.engine.check_external_changes().unwrap();
    assert_eq!(changed, vec![t]);
    assert!(e.engine.table_info(t).unwrap().externally_modified);
    e.engine.save_table(t, true).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a\n2\n");
    // A table without changes is read again automatically.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&p, "a\n7\n").unwrap();
    e.engine.check_external_changes().unwrap();
    assert_eq!(e.rows(t), vec![vec![s("7")]]);
}

#[test]
fn reports_unencodable_characters() {
    let e = env();
    let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("名前,値\n日本,1\n");
    let p = e.file("j.csv", &sjis);
    let t = e.add(&p);
    e.engine.open_table(t).unwrap();
    let run = e.sql("UPDATE j SET 名前 = '😀' WHERE 値 = 1");
    assert!(run.error.is_none(), "{:?}", run.error);
    let err = e.engine.save_table(t, false).unwrap_err();
    assert_eq!(err.code(), "UNENCODABLE");
    assert!(err.to_string().contains("名前"));
    assert_eq!(std::fs::read(&p).unwrap(), sjis.as_ref());
}

#[test]
fn runs_sql_with_lazy_loading_and_results() {
    let e = env();
    let s1 = e.file("sales.csv", "user_id,金額\n1,100\n2,200\n1,50\n".as_bytes());
    let u = e.file("users.csv", "id,部署\n1,営業\n2,開発\n".as_bytes());
    e.add(&s1);
    e.add(&u);
    let run = e.sql(
        "-- totals\nSELECT u.部署, SUM(s.金額) AS 合計 FROM sales s JOIN users u ON u.id = s.user_id GROUP BY u.部署 ORDER BY 合計 DESC;\n\
         SELECT 1 AS one, 'x' AS two",
    );
    assert!(run.error.is_none(), "{:?}", run.error);
    assert_eq!(run.results.len(), 2);
    let StatementResult::Rows {
        result_id,
        columns,
        row_count,
        ..
    } = &run.results[0]
    else {
        panic!()
    };
    assert_eq!(*row_count, 2);
    assert_eq!(columns[0].logical_type, LogicalType::Text);
    assert_eq!(columns[1].logical_type, LogicalType::Integer);
    let page = e.engine.fetch_result(*result_id, 0, 10).unwrap();
    assert_eq!(page.data[0], vec![s("開発"), s("営業")]);
    assert_eq!(page.data[1], vec![s("200"), s("150")]);
    let (_, vals, _) = e.engine.result_values(*result_id, 0, 10).unwrap();
    assert_eq!(vals[0][1], CellValue::Integer(200));

    // DML with RETURNING, and undo.
    let run = e.sql("UPDATE sales SET 金額 = 金額 * 2 WHERE user_id = 1 RETURNING 金額");
    let StatementResult::Changed {
        changed_rows,
        table_id,
        row_count,
        ..
    } = &run.results[0]
    else {
        panic!("{:?}", run)
    };
    assert_eq!((*changed_rows, *row_count), (2, 2));
    assert!(e.engine.table_meta(*table_id).unwrap().dirty);
    e.engine.undo(*table_id).unwrap();
    assert!(!e.engine.table_meta(*table_id).unwrap().dirty);

    // Insert and delete through SQL keep the row order table up to date.
    let run =
        e.sql("INSERT INTO users(id, 部署) VALUES (3, '総務'); DELETE FROM users WHERE id = 1;");
    assert!(run.error.is_none(), "{:?}", run.error);
    let users = e.engine.find_table("users").unwrap();
    assert_eq!(
        e.rows(users),
        vec![vec![s("2"), s("開発")], vec![s("3"), s("総務")]]
    );

    // Errors: position of the failing statement.
    let run = e.sql("SELECT 1; DROP TABLE sales;");
    assert_eq!(run.results.len(), 1);
    let err = run.error.unwrap();
    assert_eq!(err.code, "SQL_FORBIDDEN");
    assert_eq!(err.statement_index, 1);
    assert_eq!(err.offset, 10);

    for bad in [
        "VACUUM",
        "SELECT * FROM __arm_ord_1",
        "PRAGMA table_info(sales)",
        "ATTACH 'x.db' AS x",
        "CREATE TABLE t(a)",
        "BEGIN",
        "UPDATE sales SET rowid = 5",
    ] {
        let r = e.sql(bad);
        assert_eq!(
            r.error.as_ref().map(|e| e.code.as_str()),
            Some("SQL_FORBIDDEN"),
            "{bad}: {:?}",
            r
        );
    }
    let r = e.sql("SELECT * FROM nothing_here");
    assert_eq!(r.error.unwrap().code, "SQL_ERROR");
    let r = e.engine.run_sql(
        "DELETE FROM sales",
        &SqlOptions {
            read_only: true,
            ..Default::default()
        },
    );
    assert_eq!(r.error.unwrap().code, "SQL_FORBIDDEN");
    let r = e.sql("EXPLAIN QUERY PLAN SELECT * FROM sales");
    assert!(r.error.is_none(), "{:?}", r.error);
    let r = e.sql("SELECT * FROM sales WHERE 金額 = '100' -- comment");
    assert!(r.error.is_none(), "{:?}", r.error);
}

#[test]
fn undo_limit_asks_before_commit() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = EngineConfig {
        workspace_path: dir.path().join("workspace.json"),
        temp_dir: dir.path().join("tmp"),
        undo_max_bytes: 64,
        ..EngineConfig::default()
    };
    let engine = Engine::new(cfg, Arc::new(NoEvents)).unwrap();
    let p = dir.path().join("big.csv");
    let mut text = String::from("a\n");
    for i in 0..100 {
        text.push_str(&format!("{i}\n"));
    }
    std::fs::write(&p, &text).unwrap();
    engine.add_sources(std::slice::from_ref(&p)).unwrap();
    let r = engine.run_sql("UPDATE big SET a = a + 1", &SqlOptions::default());
    assert_eq!(r.error.unwrap().code, "UNDO_LIMIT_EXCEEDED");
    let t = engine.find_table("big").unwrap();
    assert!(!engine.table_meta(t).unwrap().dirty);
    let r = engine.run_sql(
        "UPDATE big SET a = a + 1",
        &SqlOptions {
            allow_no_undo: true,
            ..Default::default()
        },
    );
    assert!(r.error.is_none());
    let m = engine.table_meta(t).unwrap();
    assert!(m.dirty && !m.can_undo);
}

#[test]
fn views_sort_and_filter() {
    let e = env();
    let p = e.file("v.csv", b"n,name\n3,c\n1,a\n2,b\n10,ab\n");
    let t = e.add(&p);
    e.engine.open_table(t).unwrap();
    let (v, total) = e
        .engine
        .create_view(
            t,
            ViewSpec {
                sort: vec![SortKey {
                    column: "n".into(),
                    direction: SortDirection::Desc,
                }],
                filters: vec![FilterCond {
                    column: Some("name".into()),
                    op: FilterOp::Contains,
                    value: s("A"),
                }],
            },
        )
        .unwrap();
    assert_eq!(total, 2);
    let page = e.engine.fetch_rows(t, Some(v), 0, 10).unwrap();
    assert_eq!(page.data[0], vec![s("10"), s("1")]);
    assert_eq!(page.row_numbers, vec![4, 2]);
    let (v2, total) = e
        .engine
        .create_view(
            t,
            ViewSpec {
                sort: vec![],
                filters: vec![FilterCond {
                    column: Some("n".into()),
                    op: FilterOp::Greater,
                    value: s("2"),
                }],
            },
        )
        .unwrap();
    assert_eq!(total, 2);
    // A view is filled again after rows are deleted.
    let rid = e.engine.fetch_rows(t, Some(v2), 0, 1).unwrap().rids[0];
    e.engine
        .apply_edit(t, EditOp::DeleteRows { rids: vec![rid] })
        .unwrap();
    assert_eq!(e.engine.fetch_rows(t, Some(v2), 0, 10).unwrap().total, 1);
    e.engine.close_view(v).unwrap();
    assert!(e.engine.fetch_rows(t, Some(v), 0, 10).is_err());
}

#[test]
fn diff_and_export() {
    let e = env();
    let p = e.file("d.csv", b"id,v\n1,a\n2,b\n3,c\n");
    let t = e.add(&p);
    e.engine.open_table(t).unwrap();
    let run = e.sql("UPDATE d SET v = 'B' WHERE id = 2; DELETE FROM d WHERE id = 1; INSERT INTO d VALUES (4, 'd');");
    assert!(run.error.is_none(), "{:?}", run.error);
    let diff = e.engine.diff_table(t, 10).unwrap();
    assert_eq!(
        (diff.inserted_rows, diff.deleted_rows, diff.changed_rows),
        (1, 1, 1)
    );
    assert_eq!(diff.sample[0].before, s("b"));
    assert_eq!(diff.sample[0].after, s("B"));

    let out = e.dir.path().join("out.tsv");
    e.engine
        .export_table(t, &out, ExportFormat::for_path(&out))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "id\tv\r\n2\tB\r\n3\tc\r\n4\td\r\n"
    );
    let x = e.dir.path().join("out.xlsx");
    e.engine
        .export_table(
            t,
            &x,
            ExportFormat::Xlsx {
                sheet_name: Some("data".into()),
            },
        )
        .unwrap();
    let sheets = crate::ingest::excel::list_sheets(&x).unwrap();
    assert_eq!(sheets[0].name, "data");

    let run = e.sql("SELECT v, id * 1.5 AS x FROM d ORDER BY id DESC");
    let StatementResult::Rows { result_id, .. } = &run.results[0] else {
        panic!()
    };
    let scratch = e.engine.save_result_as_scratch(*result_id).unwrap();
    assert_eq!(scratch.sql_name, "scratch_01");
    let src = e
        .engine
        .list_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.id == scratch.source_id)
        .unwrap();
    assert!(src.scratch);
    let text = std::fs::read_to_string(&src.path).unwrap();
    assert_eq!(text, "v,x\nd,6.0\nc,4.5\nB,3.0\n");
}

#[test]
fn excel_sheets_are_save_as_only() {
    let e = env();
    let p = e.dir.path().join("book.xlsx");
    {
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet().set_name("2026").unwrap();
        ws.write(0, 0, "item").unwrap();
        ws.write(0, 1, "budget").unwrap();
        ws.write(1, 0, "travel").unwrap();
        ws.write(1, 1, 100).unwrap();
        ws.write_formula(1, 2, "=B2*2").unwrap();
        wb.save(&p).unwrap();
    }
    let t = e.add(&p);
    let info = e.engine.table_info(t).unwrap();
    assert_eq!(info.sql_name, "book_2026");
    let m = e.engine.open_table(t).unwrap();
    assert!(m.save_as_only);
    let err = e.engine.save_table(t, false).unwrap_err();
    assert_eq!(err.code(), "READ_ONLY");
    let page = e.engine.fetch_rows(t, None, 0, 10).unwrap();
    assert_eq!(page.notes.len(), 1);
    let err = e
        .engine
        .apply_edit(
            t,
            EditOp::SetCells {
                cells: vec![CellEdit {
                    rid: page.rids[0],
                    column: "column3".into(),
                    value: s("1"),
                }],
            },
        )
        .unwrap_err();
    assert_eq!(err.code(), "READ_ONLY");
    let out = e.dir.path().join("book_2026.csv");
    e.engine.save_table_as(t, &out, None).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    // The header of the third column is empty in the file. rust_xlsxwriter writes 0 as the cached formula value.
    assert_eq!(text, "\u{feff}item,budget,\r\ntravel,100,0\r\n");
    assert_eq!(e.engine.list_sources().unwrap().len(), 2);
}

#[test]
fn rename_and_remove_sources() {
    let e = env();
    let p = e.file("r.csv", b"a\n1\n");
    let t = e.add(&p);
    e.engine.open_table(t).unwrap();
    let info = e.engine.rename_table(t, "renamed").unwrap();
    assert_eq!(info.sql_name, "renamed");
    let run = e.sql("SELECT count(*) FROM renamed");
    assert!(run.error.is_none());
    assert_eq!(e.engine.fetch_rows(t, None, 0, 10).unwrap().total, 1);
    let sid = info.source_id;
    e.engine.remove_source(&sid).unwrap();
    assert!(e.engine.list_sources().unwrap().is_empty());
    assert!(p.exists());
    let run = e.sql("SELECT * FROM renamed");
    assert!(run.error.is_some());
}

// ----- Scratch sources (04. Workspace, section 6) -----

fn scratch_of(e: &Env, sql: &str) -> ScratchInfo {
    let run = e.sql(sql);
    let StatementResult::Rows { result_id, .. } = &run.results[0] else {
        panic!("{:?}", run.results)
    };
    e.engine.save_result_as_scratch(*result_id).unwrap()
}

fn path_of(e: &Env, scratch: &ScratchInfo) -> PathBuf {
    let src = e
        .engine
        .list_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.id == scratch.source_id)
        .unwrap();
    PathBuf::from(src.path)
}

fn scratch_env() -> (Env, ScratchInfo) {
    let e = env();
    let p = e.file("d.csv", b"id,v\n1,a\n2,b\n3,c\n");
    e.add(&p);
    let sc = scratch_of(&e, "SELECT id, v FROM d ORDER BY id");
    (e, sc)
}

#[test]
fn scratch_names_grow_and_are_queryable() {
    let (e, first) = scratch_env();
    assert_eq!(first.sql_name, "scratch_01");
    let second = scratch_of(&e, "SELECT id FROM scratch_01 WHERE id > 1");
    assert_eq!(second.sql_name, "scratch_02");
    e.engine.open_table(second.table_id).unwrap();
    let rows = e.rows(second.table_id);
    assert_eq!(rows.len(), 2);
    let dir = path_of(&e, &first).parent().unwrap().to_path_buf();
    assert!(dir.starts_with(e.dir.path().join("scratch")));
    assert!(dir.join("scratch_02.csv").is_file());
}

#[test]
fn scratch_numbers_are_not_used_again() {
    let (e, first) = scratch_env();
    let path1 = path_of(&e, &first);
    e.engine.remove_source(&first.source_id).unwrap();
    assert!(!path1.exists());
    let second = scratch_of(&e, "SELECT id FROM d");
    assert_eq!(second.sql_name, "scratch_02");

    // A new engine on the same workspace file continues the numbers.
    let cfg = EngineConfig {
        workspace_path: e.engine.workspace_path(),
        temp_dir: e.dir.path().join("tmp2"),
        scratch_root: e.dir.path().join("scratch"),
        scratch_idle: Duration::ZERO,
        ..EngineConfig::default()
    };
    let engine2 = Engine::new(cfg, Arc::new(NoEvents)).unwrap();
    let run = engine2.run_sql("SELECT id FROM d", &SqlOptions::default());
    let StatementResult::Rows { result_id, .. } = &run.results[0] else {
        panic!()
    };
    let third = engine2.save_result_as_scratch(*result_id).unwrap();
    assert_eq!(third.sql_name, "scratch_03");
}

#[test]
fn failed_result_leaves_no_scratch_file() {
    let e = env();
    assert!(e.engine.save_result_as_scratch(999).is_err());
    assert!(e.engine.list_sources().unwrap().is_empty());
    let root = e.dir.path().join("scratch");
    let files: Vec<_> = walk(&root);
    assert!(files.is_empty(), "{files:?}");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

fn set_v(e: &Env, t: TableId, value: &str) {
    let page = e.engine.fetch_rows(t, None, 0, 10).unwrap();
    e.engine
        .apply_edit(
            t,
            EditOp::SetCells {
                cells: vec![CellEdit {
                    rid: page.rids[0],
                    column: "v".into(),
                    value: s(value),
                }],
            },
        )
        .unwrap();
}

#[test]
fn scratch_tables_are_saved_automatically() {
    let (e, sc) = scratch_env();
    let path = path_of(&e, &sc);
    e.engine.open_table(sc.table_id).unwrap();
    set_v(&e, sc.table_id, "X");
    assert!(e.engine.table_meta(sc.table_id).unwrap().dirty);
    // The exit dialog does not list a scratch table.
    assert!(e.engine.dirty_tables().unwrap().is_empty());
    assert_eq!(e.engine.autosave_scratch().unwrap(), vec![sc.table_id]);
    assert!(!e.engine.table_meta(sc.table_id).unwrap().dirty);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "id,v\n1,X\n2,b\n3,c\n"
    );
}

#[test]
fn scratch_autosave_waits_for_the_idle_time() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = EngineConfig {
        workspace_path: dir.path().join("cfg").join("workspace.json"),
        temp_dir: dir.path().join("tmp"),
        scratch_root: dir.path().join("scratch"),
        scratch_idle: Duration::from_millis(150),
        ..EngineConfig::default()
    };
    let engine = Engine::new(cfg, Arc::new(NoEvents)).unwrap();
    let csv = dir.path().join("d.csv");
    std::fs::write(&csv, "id,v\n1,a\n").unwrap();
    engine.add_sources(&[csv]).unwrap();
    let run = engine.run_sql("SELECT id, v FROM d", &SqlOptions::default());
    let StatementResult::Rows { result_id, .. } = &run.results[0] else {
        panic!()
    };
    let sc = engine.save_result_as_scratch(*result_id).unwrap();
    engine.open_table(sc.table_id).unwrap();
    let page = engine.fetch_rows(sc.table_id, None, 0, 10).unwrap();
    engine
        .apply_edit(
            sc.table_id,
            EditOp::SetCells {
                cells: vec![CellEdit {
                    rid: page.rids[0],
                    column: "v".into(),
                    value: s("X"),
                }],
            },
        )
        .unwrap();
    assert!(engine.autosave_scratch().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(engine.autosave_scratch().unwrap(), vec![sc.table_id]);
}

#[test]
fn scratch_autosave_failure_makes_the_table_dirty() {
    let (e, sc) = scratch_env();
    let path = path_of(&e, &sc);
    e.engine.open_table(sc.table_id).unwrap();
    // Another program changes the file.
    std::fs::write(&path, "id,v\n1,other\n").unwrap();
    set_v(&e, sc.table_id, "X");
    assert!(e.engine.autosave_scratch().unwrap().is_empty());
    let info = e.engine.table_info(sc.table_id).unwrap();
    assert!(info.auto_save_failed);
    assert_eq!(e.engine.dirty_tables().unwrap().len(), 1);
    // The file is not overwritten.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "id,v\n1,other\n");
}

#[test]
fn scratch_save_as_moves_the_file() {
    let (e, sc) = scratch_env();
    let old = path_of(&e, &sc);
    e.engine.open_table(sc.table_id).unwrap();
    set_v(&e, sc.table_id, "X");
    let target = e.dir.path().join("kept.csv");
    e.engine.save_table_as(sc.table_id, &target, None).unwrap();
    assert!(target.is_file());
    assert!(!old.exists());
    let src = e
        .engine
        .list_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.id == sc.source_id)
        .unwrap();
    assert!(!src.scratch);
    assert_eq!(PathBuf::from(&src.path), target);
    // The SQL name does not change.
    assert_eq!(src.tables[0].sql_name, "scratch_01");
    assert!(std::fs::read_to_string(&target).unwrap().contains("1,X"));
    // The source is a normal source now: removing it keeps the file.
    e.engine.remove_source(&sc.source_id).unwrap();
    assert!(target.is_file());
}

#[test]
fn scratch_delete_removes_the_file_and_relocate_is_refused() {
    let (e, sc) = scratch_env();
    let path = path_of(&e, &sc);
    let other = e.file("o.csv", b"a\n1\n");
    let err = e.engine.relocate_source(&sc.source_id, &other).unwrap_err();
    assert_eq!(err.code(), "INVALID_ARGUMENT");
    e.engine.remove_source(&sc.source_id).unwrap();
    assert!(!path.exists());
    assert!(other.is_file());
}

#[test]
fn scratch_cleanup_deletes_only_unregistered_scratch_files() {
    let (e, sc) = scratch_env();
    let path = path_of(&e, &sc);
    let dir = path.parent().unwrap();
    let orphan = dir.join("scratch_09.csv");
    let temp = dir.join(".scratch_10.csv.armarium-1-x.tmp");
    let other = dir.join("notes.csv");
    for p in [&orphan, &temp, &other] {
        std::fs::write(p, "a\n").unwrap();
    }
    assert_eq!(e.engine.cleanup_scratch().unwrap(), 2);
    assert!(!orphan.exists());
    assert!(!temp.exists());
    assert!(other.exists());
    assert!(path.exists());
}

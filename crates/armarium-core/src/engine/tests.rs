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
    let saved = e.dir.path().join("result.csv");
    let sid = e
        .engine
        .save_result_as_source(*result_id, &saved, ExportFormat::for_path(&saved))
        .unwrap();
    assert!(!sid.is_empty());
    let text = std::fs::read_to_string(&saved).unwrap();
    assert_eq!(text, "\u{feff}v,x\r\nd,6.0\r\nc,4.5\r\nB,3.0\r\n");
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

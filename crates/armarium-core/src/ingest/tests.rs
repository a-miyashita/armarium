use super::*;
use crate::db::{open_write, DbSettings};
use std::sync::atomic::AtomicBool;

fn ctl_run<T>(f: impl FnOnce(&LoadControl<'_>) -> T) -> T {
    let cancel = AtomicBool::new(false);
    let progress = |_: u64, _: Option<u64>| {};
    f(&LoadControl {
        cancel: &cancel,
        progress: &progress,
    })
}

fn setup() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_write(&dir.path().join("s.db"), &DbSettings::default()).unwrap();
    (dir, conn)
}

#[test]
fn loads_csv_with_round_trip_safe_types() {
    let (dir, conn) = setup();
    let p = dir.path().join("sales.csv");
    std::fs::write(
        &p,
        "id,zip,price,ratio,date,ts,name\r\n\
         1,00123,1.50,0.5,2026-01-02,2026-01-02 10:00:00,Apple\r\n\
         2,00456,2.00,1.25,2026-02-28,2026-01-03 11:30:00,\"Banana, ripe\"\r\n\
         3,,3.10,,2026-03-01,,\r\n",
    )
    .unwrap();
    let l = ctl_run(|c| {
        load_csv(
            &conn,
            1,
            "sales",
            &p,
            &ReadOptions {
                type_inference: true,
                ..Default::default()
            },
            c,
        )
    })
    .unwrap();
    let types: Vec<LogicalType> = l.columns.iter().map(|c| c.logical_type).collect();
    assert_eq!(
        types,
        vec![
            LogicalType::Integer,
            LogicalType::Text,
            LogicalType::Text,
            LogicalType::Real,
            LogicalType::Date,
            LogicalType::DateTime,
            LogicalType::Text
        ]
    );
    assert_eq!(l.row_count, 3);
    let d = l.dialect.unwrap();
    assert_eq!(d.line_ending, crate::model::LineEnding::Crlf);
    assert!(d.has_header);
    let (zip, ty): (Option<String>, String) = conn
        .query_row("SELECT zip, typeof(id) FROM sales WHERE id = 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(zip.as_deref(), Some("00123"));
    assert_eq!(ty, "integer");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM __arm_ord_1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 3);
    let name: Option<String> = conn
        .query_row("SELECT name FROM sales WHERE id = 3", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, None);
}

#[test]
fn loads_ragged_rows_without_data_loss() {
    let (dir, conn) = setup();
    let p = dir.path().join("r.csv");
    std::fs::write(&p, "a,b\n1,2\n3\n4,5,6\n").unwrap();
    let l = ctl_run(|c| {
        load_csv(
            &conn,
            2,
            "r",
            &p,
            &ReadOptions {
                type_inference: true,
                ..Default::default()
            },
            c,
        )
    })
    .unwrap();
    assert_eq!(l.columns.len(), 3);
    assert_eq!(l.columns[2].sql_name, "column3");
    assert_eq!(l.warnings.len(), 1);
    let v: i64 = conn
        .query_row("SELECT column3 FROM r WHERE a = 4", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 6);
}

#[test]
fn applies_type_overrides_and_disables_inference() {
    let (dir, conn) = setup();
    let p = dir.path().join("o.csv");
    std::fs::write(&p, "d,n\n2026/01/02,1\n2026/1/3,2\n").unwrap();
    let mut opts = ReadOptions {
        type_inference: true,
        ..Default::default()
    };
    opts.column_types.insert("d".into(), LogicalType::Date);
    let l = ctl_run(|c| load_csv(&conn, 3, "o", &p, &opts, c)).unwrap();
    assert_eq!(l.columns[0].logical_type, LogicalType::Date);
    assert_eq!(l.columns[0].type_source, TypeSource::UserOverride);
    let v: String = conn
        .query_row("SELECT d FROM o WHERE n = 2", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, "2026-01-03");

    let opts = ReadOptions {
        type_inference: false,
        ..Default::default()
    };
    let l = ctl_run(|c| load_csv(&conn, 3, "o", &p, &opts, c)).unwrap();
    assert!(l
        .columns
        .iter()
        .all(|c| c.logical_type == LogicalType::Text));
}

#[test]
fn loads_shift_jis_and_skips_rows() {
    let (dir, conn) = setup();
    let p = dir.path().join("j.csv");
    let (bytes, _, _) =
        encoding_rs::SHIFT_JIS.encode("タイトル行\r\n部署,金額\r\n営業,1200000\r\n開発,800000\r\n");
    std::fs::write(&p, &bytes).unwrap();
    let opts = ReadOptions {
        type_inference: true,
        skip_rows: 1,
        ..Default::default()
    };
    let l = ctl_run(|c| load_csv(&conn, 4, "j", &p, &opts, c)).unwrap();
    let d = l.dialect.unwrap();
    assert_eq!(d.encoding, "Shift_JIS");
    assert_eq!(d.skipped_lines, vec!["タイトル行".to_string()]);
    assert_eq!(l.columns[0].sql_name, "部署");
    let total: i64 = conn
        .query_row("SELECT sum(金額) FROM j", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 2_000_000);
}

#[test]
fn loads_excel_sheets() {
    let (dir, conn) = setup();
    let p = dir.path().join("budget.xlsx");
    {
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet().set_name("2026").unwrap();
        ws.write(0, 0, "科目").unwrap();
        ws.write(0, 1, "予算").unwrap();
        ws.write(0, 2, "日付").unwrap();
        ws.write(0, 3, "合計").unwrap();
        ws.write(1, 0, "旅費").unwrap();
        ws.write(1, 1, 100).unwrap();
        let fmt = rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd");
        let date = rust_xlsxwriter::ExcelDateTime::from_ymd(2026, 1, 2).unwrap();
        ws.write_datetime_with_format(1, 2, &date, &fmt).unwrap();
        ws.write_formula(1, 3, "=B2*2").unwrap();
        ws.write(2, 0, "会議費").unwrap();
        ws.write(2, 1, 50.5).unwrap();
        let hidden = wb.add_worksheet().set_name("old").unwrap();
        hidden.write(0, 0, "x").unwrap();
        hidden.set_hidden(true);
        wb.save(&p).unwrap();
    }
    let sheets = excel::list_sheets(&p).unwrap();
    assert_eq!(sheets.len(), 2);
    assert!(sheets[1].hidden);
    let opts = ReadOptions {
        type_inference: true,
        ..Default::default()
    };
    let l = ctl_run(|c| excel::load_sheet(&conn, 5, "budget_2026", &p, "2026", &opts, c)).unwrap();
    let types: Vec<LogicalType> = l.columns.iter().map(|c| c.logical_type).collect();
    assert_eq!(
        types[..3],
        [LogicalType::Text, LogicalType::Real, LogicalType::Date]
    );
    assert_eq!(l.row_count, 2);
    let d: String = conn
        .query_row("SELECT 日付 FROM budget_2026 WHERE rowid = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(d, "2026-01-02");
    assert!(l
        .notes
        .get(&(1, "合計".to_string()))
        .map(|(k, t)| *k == CellNoteKind::Formula && t == "=B2*2")
        .unwrap_or(false));
}

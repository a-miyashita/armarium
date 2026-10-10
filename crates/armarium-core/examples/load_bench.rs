//! Spike M0-8: measures the time to read a large CSV file and to use it.
//!
//! Usage: `cargo run --release -p armarium-core --example load_bench -- [rows] [path]`
//! The example makes the file if it does not exist (M1-11: a script for large test files).

use armarium_core::sql::SqlOptions;
use armarium_core::{Engine, EngineConfig, NoEvents};
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rows: u64 = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let dir = tempfile::tempdir().unwrap();
    let path = args
        .get(2)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir.path().join("big.csv"));
    if !path.exists() {
        let t = Instant::now();
        let mut w = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        writeln!(w, "id,name,category,amount,price,date,note").unwrap();
        for i in 0..rows {
            writeln!(
                w,
                "{i},name_{},{},{},{}.{:02},2026-{:02}-{:02},\"note, with comma {}\"",
                i % 10007,
                ["food", "tools", "books", "toys"][(i % 4) as usize],
                (i * 7919) % 100000,
                i % 1000,
                i % 100,
                1 + i % 12,
                1 + i % 28,
                i % 997
            )
            .unwrap();
        }
        w.flush().unwrap();
        println!("generated {} rows in {:?}", rows, t.elapsed());
    }
    let size = std::fs::metadata(&path).unwrap().len();
    println!("file: {} ({:.1} MB)", path.display(), size as f64 / 1e6);

    let cfg = EngineConfig {
        workspace_path: dir.path().join("workspace.json"),
        temp_dir: dir.path().join("tmp"),
        ..EngineConfig::default()
    };
    let engine = Engine::new(cfg, Arc::new(NoEvents)).unwrap();
    let r = engine.add_sources(&[path.clone()]).unwrap();
    let sid = r[0].source_id.clone().unwrap();
    let table = engine
        .list_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.id == sid)
        .unwrap()
        .tables[0]
        .id;

    let t = Instant::now();
    let meta = engine.open_table(table).unwrap();
    println!(
        "open_table: {:?} ({} rows, types {:?})",
        t.elapsed(),
        meta.row_count,
        meta.columns
            .iter()
            .map(|c| c.logical_type.as_str())
            .collect::<Vec<_>>()
    );

    let t = Instant::now();
    let page = engine
        .fetch_rows(table, None, meta.row_count * 9 / 10, 200)
        .unwrap();
    println!(
        "fetch_rows near 90%: {:?} ({} rows)",
        t.elapsed(),
        page.rids.len()
    );

    let t = Instant::now();
    let run = engine.run_sql(
        &format!(
            "SELECT category, sum(amount), avg(price), count(*) FROM {} GROUP BY category",
            meta.sql_name
        ),
        &SqlOptions::default(),
    );
    assert!(run.error.is_none(), "{:?}", run.error);
    println!("GROUP BY: {:?}", t.elapsed());

    let t = Instant::now();
    let (view, n) = engine
        .create_view(
            table,
            armarium_core::model::ViewSpec {
                sort: vec![armarium_core::model::SortKey {
                    column: "amount".into(),
                    direction: armarium_core::model::SortDirection::Desc,
                }],
                filters: vec![],
            },
        )
        .unwrap();
    println!("sort view: {:?} ({n} rows)", t.elapsed());
    let t = Instant::now();
    engine
        .fetch_rows(table, Some(view), n * 9 / 10, 200)
        .unwrap();
    println!("fetch sorted near 90%: {:?}", t.elapsed());

    let t = Instant::now();
    let run = engine.run_sql(
        &format!("UPDATE {} SET note = 'x' WHERE id % 10 = 0", meta.sql_name),
        &SqlOptions::default(),
    );
    assert!(run.error.is_none(), "{:?}", run.error);
    println!("UPDATE 10% with undo: {:?}", t.elapsed());
    let t = Instant::now();
    engine.undo(table).unwrap();
    println!("undo: {:?}", t.elapsed());

    let t = Instant::now();
    engine.save_table(table, true).unwrap();
    println!("save: {:?}", t.elapsed());
}

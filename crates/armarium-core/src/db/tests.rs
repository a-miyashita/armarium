//! Spikes M0-5 (session extension with rowid tables) and M0-7 (authorizer).

use super::auth::Sandbox;
use super::changeset::{self, Recorder};
use super::*;

fn setup() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_write(&dir.path().join("s.db"), &DbSettings::default()).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE "sales" ("id" INTEGER, "name" TEXT) STRICT;
        CREATE TABLE "__arm_ord_1" (rid INTEGER PRIMARY KEY, ord REAL NOT NULL);
        CREATE INDEX "__arm_ord_1_ix" ON "__arm_ord_1"(ord);
        CREATE TRIGGER "__arm_ord_1_ins" AFTER INSERT ON "sales"
          WHEN NOT EXISTS (SELECT 1 FROM "__arm_ord_1" WHERE rid = NEW.rowid)
          BEGIN INSERT INTO "__arm_ord_1"(rid, ord)
            VALUES (NEW.rowid, coalesce((SELECT max(ord) FROM "__arm_ord_1"), 0) + 1); END;
        CREATE TRIGGER "__arm_ord_1_del" AFTER DELETE ON "sales"
          BEGIN DELETE FROM "__arm_ord_1" WHERE rid = OLD.rowid; END;
        INSERT INTO sales(id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c');
        "#,
    )
    .unwrap();
    (dir, conn)
}

fn dump(conn: &Connection) -> Vec<(i64, i64, String, f64)> {
    let mut st = conn
        .prepare(
            "SELECT t.rowid, t.id, t.name, o.ord FROM sales t JOIN __arm_ord_1 o ON o.rid = t.rowid ORDER BY o.ord",
        )
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn record(conn: &Connection, sql: &str) -> Vec<u8> {
    let rec = Recorder::start(conn, &["sales", "__arm_ord_1"]).unwrap();
    conn.execute_batch(&format!("BEGIN; {sql}; COMMIT;"))
        .unwrap();
    rec.changeset().unwrap()
}

#[test]
fn session_undo_and_redo_with_rowid_tables() {
    let (_d, conn) = setup();
    let before = dump(&conn);

    // Delete a row in the middle, then undo: the row returns with the same rowid and order.
    let cs = record(&conn, "DELETE FROM sales WHERE id = 2");
    assert!(!cs.is_empty());
    assert_eq!(dump(&conn).len(), 2);
    changeset::apply(&conn, &changeset::invert(&cs).unwrap()).unwrap();
    assert_eq!(dump(&conn), before);
    // Redo.
    changeset::apply(&conn, &cs).unwrap();
    assert_eq!(dump(&conn).len(), 2);
    changeset::apply(&conn, &changeset::invert(&cs).unwrap()).unwrap();

    // Insert in the middle (trigger + order update), then undo.
    let cs = record(
        &conn,
        "INSERT INTO sales(id, name) VALUES (9, 'x'); UPDATE __arm_ord_1 SET ord = 1.5 WHERE rid = last_insert_rowid()",
    );
    let after = dump(&conn);
    assert_eq!(after[1].1, 9);
    changeset::apply(&conn, &changeset::invert(&cs).unwrap()).unwrap();
    assert_eq!(dump(&conn), before);
    changeset::apply(&conn, &cs).unwrap();
    assert_eq!(dump(&conn), after);

    // Update, then undo.
    let cs = record(&conn, "UPDATE sales SET name = 'zz' WHERE id = 1");
    changeset::apply(&conn, &changeset::invert(&cs).unwrap()).unwrap();
    assert_eq!(dump(&conn), after);
}

fn check(conn: &Connection, sb: &Sandbox, sql: &str) -> std::result::Result<Vec<String>, String> {
    sb.begin(["sales".to_string()], false);
    let r = conn.prepare(sql).map(|_| ());
    let rep = sb.report();
    sb.end();
    match r {
        Ok(()) => Ok(rep.writes),
        Err(e) => Err(rep.denied.unwrap_or_else(|| e.to_string())),
    }
}

#[test]
fn authorizer_allows_queries_and_dml() {
    let (_d, conn) = setup();
    let sb = Sandbox::install(&conn).unwrap();
    assert_eq!(
        check(&conn, &sb, "SELECT * FROM sales WHERE name REGEXP 'a'"),
        Ok(vec![])
    );
    assert_eq!(
        check(&conn, &sb, "WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM r WHERE n<3) SELECT n FROM r"),
        Ok(vec![])
    );
    assert_eq!(
        check(
            &conn,
            &sb,
            "SELECT name, sum(id) OVER (ORDER BY id) FROM sales"
        ),
        Ok(vec![])
    );
    assert_eq!(
        check(&conn, &sb, "INSERT INTO sales(id, name) VALUES (5, 'e')"),
        Ok(vec!["sales".to_string()])
    );
    assert_eq!(
        check(
            &conn,
            &sb,
            "UPDATE sales SET name = upper(name) WHERE id > 1 RETURNING *"
        ),
        Ok(vec!["sales".to_string()])
    );
    assert_eq!(
        check(&conn, &sb, "DELETE FROM sales"),
        Ok(vec!["sales".to_string()])
    );
    assert!(check(&conn, &sb, "SELECT * FROM json_each('[1,2]')").is_ok());
    assert!(check(&conn, &sb, "EXPLAIN QUERY PLAN SELECT * FROM sales").is_ok());
}

#[test]
fn authorizer_refuses_dangerous_statements() {
    let (_d, conn) = setup();
    let sb = Sandbox::install(&conn).unwrap();
    for sql in [
        "CREATE TABLE x(a)",
        "CREATE TEMP VIEW v AS SELECT 1",
        "DROP TABLE sales",
        "ALTER TABLE sales ADD COLUMN z",
        "ATTACH 'x.db' AS x",
        "PRAGMA journal_mode",
        "SELECT * FROM pragma_table_info('sales')",
        "BEGIN",
        "SAVEPOINT a",
        "ANALYZE",
        "REINDEX",
        "SELECT * FROM __arm_ord_1",
        "DELETE FROM __arm_ord_1",
        "SELECT * FROM sqlite_schema",
        "UPDATE sales SET rowid = 10",
        "SELECT load_extension('x')",
        "SELECT fts3_tokenizer('x')",
        "CREATE VIRTUAL TABLE f USING fts5(a)",
        "SELECT * FROM dbstat",
    ] {
        assert!(check(&conn, &sb, sql).is_err(), "must refuse: {sql}");
    }
    // VACUUM has no authorizer code. The statement classifier refuses it
    // (a statement that is not read-only must write exactly one working table).
    sb.begin(["sales".to_string()], false);
    let st = conn.prepare("VACUUM").unwrap();
    assert!(!st.readonly());
    assert!(sb.report().writes.is_empty());
    sb.end();
}

#[test]
fn authorizer_read_only_mode_refuses_dml() {
    let (_d, conn) = setup();
    let sb = Sandbox::install(&conn).unwrap();
    sb.begin(["sales".to_string()], true);
    assert!(conn.prepare("DELETE FROM sales").is_err());
    assert!(conn.prepare("SELECT * FROM sales").is_ok());
    sb.end();
}

#[test]
fn functions_work() {
    let (_d, conn) = setup();
    let v: (bool, String, String) = conn
        .query_row(
            "SELECT 'abc' REGEXP '^a.c$', regexp_replace('a1b2', '[0-9]', '#'), normalize_nfkc('ＡＢＣ１２３ｱ')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(v, (true, "a#b#".into(), "ABC123ア".into()));
}

#[test]
fn round_trip_cast_rules() {
    // ADR-0007 depends on the text format of CAST in SQLite.
    let conn = Connection::open_in_memory().unwrap();
    let ok = |sql: &str| -> bool { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert!(ok("SELECT CAST(CAST('123' AS INTEGER) AS TEXT) = '123'"));
    assert!(!ok(
        "SELECT CAST(CAST('00123' AS INTEGER) AS TEXT) = '00123'"
    ));
    assert!(ok("SELECT CAST(CAST('1.5' AS REAL) AS TEXT) = '1.5'"));
    assert!(!ok("SELECT CAST(CAST('1.50' AS REAL) AS TEXT) = '1.50'"));
    assert!(ok("SELECT CAST(CAST('0.1' AS REAL) AS TEXT) = '0.1'"));
    assert!(ok("SELECT date(julianday('2026-01-02')) = '2026-01-02'"));
    assert!(ok(
        "SELECT date(julianday('2026-02-30')) IS NOT '2026-02-30'"
    ));
}

#![allow(clippy::unwrap_used, clippy::expect_used)] // test code
use logbox_core::{ch, pg, testing};

#[tokio::test]
async fn migrations_apply_twice_and_reader_is_read_only() {
    let Some(env) = testing::env().await else {
        eprintln!("skipping: LOGBOX_TEST_* not set");
        return;
    };
    let cfg = &env.config;

    // ClickHouse: idempotent, and re-running after a retention change works.
    ch::migrate(cfg).await.unwrap();
    ch::migrate(cfg).await.unwrap();
    let mut changed = cfg.clone();
    changed.retention.days = 14;
    changed.retention.body_days = 3;
    ch::migrate(&changed).await.unwrap();

    let clients = ch::ClickHouse::new(cfg);
    clients.ping().await.unwrap();
    let tables: Vec<String> = clients
        .writer
        .query("SELECT name FROM system.tables WHERE database = currentDatabase() ORDER BY name")
        .fetch_all()
        .await
        .unwrap();
    for t in [
        "requests",
        "spans",
        "events",
        "route_minute",
        "error_groups",
        "error_minute",
    ] {
        assert!(
            tables.iter().any(|n| n == t),
            "missing table {t}: {tables:?}"
        );
    }
    let ddl: String = clients
        .writer
        .query("SELECT create_table_query FROM system.tables WHERE database = currentDatabase() AND name = 'requests'")
        .fetch_one()
        .await
        .unwrap();
    assert!(ddl.contains("toIntervalDay(14)"), "{ddl}");
    assert!(ddl.contains("toIntervalDay(3)"), "{ddl}");

    // The reader can SELECT but not write, and can't escape its database.
    let n: u64 = clients
        .reader
        .query("SELECT count() FROM requests")
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(n, 0);
    let write = clients
        .reader
        .query("INSERT INTO requests (project_id) VALUES ('x')")
        .execute()
        .await;
    assert!(write.is_err(), "reader must not be able to insert");
    let drop = clients.reader.query("DROP TABLE requests").execute().await;
    assert!(drop.is_err(), "reader must not be able to drop");
    let other = clients
        .reader
        .query("SELECT count() FROM system.users")
        .fetch_one::<u64>()
        .await;
    assert!(other.is_err(), "reader must not read system.users");
    let raise = clients
        .reader
        .query("SELECT 1")
        .with_setting("max_execution_time", "3600")
        .fetch_one::<u8>()
        .await;
    assert!(raise.is_err(), "reader must not raise its limits");

    // Postgres.
    let pool = pg::connect(&cfg.postgres).await.unwrap();
    pg::migrate(&pool).await.unwrap();
    pg::migrate(&pool).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM projects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

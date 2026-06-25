//! Schema-migration tests (task 2.8): the migration produces the full schema
//! and is safe to re-apply. Requires `TEST_DATABASE_URL` (or `DATABASE_URL`).

use sqlx::postgres::PgPoolOptions;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

#[tokio::test]
async fn migrations_produce_full_schema_and_are_reappliable() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&test_db_url())
        .await
        .expect("connect");

    // Apply, then re-apply: the second run must be a no-op, not an error.
    hub::MIGRATOR.run(&pool).await.expect("first apply");
    hub::MIGRATOR
        .run(&pool)
        .await
        .expect("re-apply is idempotent");

    // All expected tables exist.
    for table in ["machines", "projects", "sessions", "messages"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "table {table} should exist after migration");
    }

    // The GIN full-text index exists.
    let gin: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = 'public' \
         AND indexname = 'messages_text_search_idx')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(gin, "GIN text_search index should exist");
}

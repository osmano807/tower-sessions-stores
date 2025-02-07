use async_trait::async_trait;
use sqlx::MySqlPool;
use time::OffsetDateTime;
use tower_sessions_core::{
    session::{Id, Record},
    session_store, ExpiredDeletion, SessionStore,
};

use crate::SqlxStoreError;

/// A MySQL session store.
#[derive(Clone, Debug)]
pub struct MySqlStore {
    pool: MySqlPool,
    schema_name: String,
    table_name: String,
}

impl MySqlStore {
    /// Create a new MySqlStore store with the provided connection pool.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions_sqlx::{sqlx::MySqlPool, MySqlStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let pool = MySqlPool::connect(database_url).await.unwrap();
    /// let session_store = MySqlStore::new(pool);
    /// # })
    /// ```
    pub fn new(pool: MySqlPool) -> Self {
        Self {
            pool,
            schema_name: "tower_sessions".to_string(),
            table_name: "session".to_string(),
        }
    }

    /// Migrate the session schema.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tower_sessions_sqlx::{sqlx::MySqlPool, MySqlStore};
    ///
    /// # tokio_test::block_on(async {
    /// let database_url = std::option_env!("DATABASE_URL").unwrap();
    /// let pool = MySqlPool::connect(database_url).await.unwrap();
    /// let session_store = MySqlStore::new(pool);
    /// session_store.migrate().await.unwrap();
    /// # })
    /// ```
    pub async fn migrate(&self) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;

        let create_schema_query = format!(
            "create schema if not exists {schema_name}",
            schema_name = self.schema_name,
        );
        sqlx::query(&create_schema_query).execute(&mut *tx).await?;

        let create_table_query = format!(
            r#"
            create table if not exists `{schema_name}`.`{table_name}`
            (
                id char(22) primary key not null,
                data blob not null,
                expiry_date timestamp(6) not null
            )
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&create_table_query).execute(&mut *tx).await?;

        tx.commit().await?;

        Ok(())
    }
}

#[async_trait]
impl ExpiredDeletion for MySqlStore {
    async fn delete_expired(&self) -> session_store::Result<()> {
        let query = format!(
            r#"
            delete from `{schema_name}`.`{table_name}`
            where expiry_date < utc_timestamp()
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .map_err(SqlxStoreError::Sqlx)?;
        Ok(())
    }
}

#[async_trait]
impl SessionStore for MySqlStore {
    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let query = format!(
            r#"
            insert into `{schema_name}`.`{table_name}`
              (id, data, expiry_date) values (?, ?, ?)
            on duplicate key update
              data = values(data),
              expiry_date = values(expiry_date)
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&query)
            .bind(&record.id.to_string())
            .bind(rmp_serde::to_vec(&record).map_err(SqlxStoreError::Encode)?)
            .bind(record.expiry_date)
            .execute(&self.pool)
            .await
            .map_err(SqlxStoreError::Sqlx)?;

        Ok(())
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        let query = format!(
            r#"
            select data from `{schema_name}`.`{table_name}`
            where id = ? and expiry_date > ?
            "#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        let data: Option<(Vec<u8>,)> = sqlx::query_as(&query)
            .bind(session_id.to_string())
            .bind(OffsetDateTime::now_utc())
            .fetch_optional(&self.pool)
            .await
            .map_err(SqlxStoreError::Sqlx)?;

        if let Some((data,)) = data {
            Ok(Some(
                rmp_serde::from_slice(&data).map_err(SqlxStoreError::Decode)?,
            ))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        let query = format!(
            r#"delete from `{schema_name}`.`{table_name}` where id = ?"#,
            schema_name = self.schema_name,
            table_name = self.table_name
        );
        sqlx::query(&query)
            .bind(&session_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(SqlxStoreError::Sqlx)?;

        Ok(())
    }
}

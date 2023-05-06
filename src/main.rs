use anyhow::{Context, Result};
use config::Config;
use notion::ids::DatabaseId;
use notion::models::paging::Pageable;
use notion::models::search::{DatabaseQuery, NotionSearch};
use notion::NotionApi;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;

#[derive(Deserialize, Serialize)]
struct AutoConfig {
    api_token: Option<String>,
    task_database_id: Option<DatabaseId>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    /*let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);*/

    let config = Config::builder()
        .add_source(config::File::with_name("notion_config"))
        .add_source(config::Environment::with_prefix("NOTION"))
        .build()?;

    let config: AutoConfig = config.try_deserialize().context("Failed to read config")?;

    let notion_api = NotionApi::new(
        std::env::var("NOTION_API_TOKEN")
            .or(config
                .api_token
                .ok_or(anyhow::anyhow!("No api token from config")))
            .context(
                "No Notion API token found in either the environment variable \
                `NOTION_API_TOKEN` or the config file!",
            )?,
    )?;

    backup_all(&notion_api).await
}

async fn dump_all<Q>(
    api: &NotionApi,
    mut f: impl FnMut(notion::models::Page),
    database: &DatabaseId,
    query: Q,
) -> Result<()>
where
    Q: Into<DatabaseQuery>,
{
    let mut q: DatabaseQuery = query.into();
    let mut pages = api.query_database(database, q.clone()).await?;
    pages.results.into_iter().for_each(&mut f);

    while pages.has_more {
        q = q.start_from(pages.next_cursor);

        pages = api.query_database(database, q.clone()).await?;
        pages.results.into_iter().for_each(&mut f);
    }

    Ok(())
}

async fn backup_all(api: &NotionApi) -> Result<()> {
    let databases = api
        .search(NotionSearch::filter_by_databases())
        .await?
        .only_databases();

    std::fs::create_dir_all("databases/")?;
    std::fs::create_dir_all("pages/")?;

    for database in databases.results().iter() {
        let title = database.title_plain_text();
        tracing::info!(
            id = database.id.to_string(),
            title = title,
            "Found Database"
        );

        {
            let mut output =
                File::create("databases/".to_string() + &database.id.to_string() + ".json")
                    .expect("File open failed!");

            let buf: String = serde_json::to_string(&database).expect("JSON serialization failed!");
            output.write_all(buf.as_bytes()).expect("Write failed");
        }

        dump_all(
            api,
            |page| {
                tracing::info!(id = page.id.to_string(), title = page.title(), "Found Page");

                let mut output =
                    File::create("pages/".to_string() + &page.id.to_string() + ".json")
                        .expect("File open failed!");

                let buf: String = serde_json::to_string(&page).expect("JSON serialization failed!");
                output.write_all(buf.as_bytes()).expect("Write failed");
            },
            &database.id,
            DatabaseQuery {
                sorts: None,
                filter: None,
                paging: None,
            },
        )
        .await?;
    }

    Ok(())
}

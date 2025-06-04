use anyhow::{Context, Result};
use config::Config;
use notion::NotionApi;
use notion::ids::{BlockId, DatabaseId};
use notion::models::paging::Pageable;
use notion::models::search::{DatabaseQuery, NotionSearch};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Serialize)]
struct AutoConfig {
    api_token: Option<String>,
    target: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(true)
        .init();

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

    backup_all(&notion_api, &config.target.unwrap_or(PathBuf::from(r"."))).await
}

async fn dump_page(api: &NotionApi, pagepath: &Path, mut page: notion::models::Page) -> Result<()> {
    tracing::info!(id = page.id.to_string(), title = page.title(), "Found Page");

    let block_id: BlockId = BlockId::from(page.id.clone());
    if let Ok(blocks) = api.get_block_children(&block_id).await {
        page.blocks = Some(blocks.results);
        let mut more = blocks.has_more;
        let mut next = blocks.next_cursor;

        while more {
            let blocks = api
                .get_block_children_with_cursor(&block_id, next.unwrap())
                .await?;
            if let Some(v) = &mut page.blocks {
                v.extend(blocks.results);
            }

            more = blocks.has_more;
            next = blocks.next_cursor;
        }
    }

    let mut output = File::create(pagepath.join(page.id.to_string() + ".json"))?;

    let buf: String = serde_json::to_string(&page)?;
    output.write_all(buf.as_bytes())?;

    Ok(())
}

async fn dump_all<Q>(
    api: &NotionApi,
    pagepath: &Path,
    database: &DatabaseId,
    query: Q,
) -> Result<()>
where
    Q: Into<DatabaseQuery>,
{
    let mut q: DatabaseQuery = query.into();
    let mut pages = api.query_database(database, q.clone()).await?;
    for page in pages.results.into_iter() {
        dump_page(api, pagepath, page).await?;
    }

    while pages.has_more {
        q = q.start_from(pages.next_cursor);

        pages = api.query_database(database, q.clone()).await?;
        for page in pages.results.into_iter() {
            dump_page(api, pagepath, page).await?;
        }
    }

    Ok(())
}

async fn backup_all(api: &NotionApi, target: &Path) -> Result<()> {
    let databases = api
        .search(NotionSearch::filter_by_databases())
        .await?
        .only_databases();

    let dbpath = target.join("databases");
    let pagepath = target.join("pages");
    tracing::info!(
        DBPath = dbpath.to_str().unwrap(),
        PagePath = pagepath.to_str().unwrap(),
        "Creating directories..."
    );

    std::fs::create_dir_all(&dbpath)?;
    std::fs::create_dir_all(&pagepath)?;

    for database in databases.results().iter() {
        let title = database.title_plain_text();
        tracing::info!(
            id = database.id.to_string(),
            title = title,
            "Found Database"
        );

        {
            let mut output = File::create(dbpath.join(database.id.to_string() + ".json"))?;

            let buf: String = serde_json::to_string(&database)?;
            output.write_all(buf.as_bytes())?;
        }

        dump_all(
            api,
            &pagepath,
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

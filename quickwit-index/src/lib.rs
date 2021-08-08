// Quickwit
//  Copyright (C) 2021 Quickwit Inc.
//
//  Quickwit is offered under the AGPL v3.0 and as commercial software.
//  For commercial licensing, contact us at hello@quickwit.io.
//
//  AGPL:
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Affero General Public License as
//  published by the Free Software Foundation, either version 3 of the
//  License, or (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Affero General Public License for more details.
//
//  You should have received a copy of the GNU Affero General Public License
//  along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::path::Path;
use std::sync::Arc;

use crate::actors::source::FileSource;
use crate::actors::Indexer;
use crate::actors::Packager;
use crate::actors::Publisher;
use crate::actors::Uploader;
use crate::models::CommitPolicy;
use quickwit_actors::AsyncActor;
use quickwit_actors::KillSwitch;
use quickwit_actors::SyncActor;
use quickwit_metastore::Metastore;
use quickwit_storage::StorageUriResolver;
use tracing::info;

pub mod actors;
pub mod models;
pub(crate) mod semaphore;

pub async fn spawn_indexing_pipeline(
    index_id: String,
    metastore: Arc<dyn Metastore>,
    storage_uri_resolver: StorageUriResolver,
) -> anyhow::Result<()> {
    info!(index_id=%index_id, "start-indexing-pipeline");
    let index_metadata = metastore.index_metadata(&index_id).await?;
    let index_storage = storage_uri_resolver.resolve(&index_metadata.index_uri)?;

    // TODO supervisor with respawn, one for all and respawn system.
    // TODO add a supervisition that checks the progress of all of these actors.
    let kill_switch = KillSwitch::default();
    let publisher = Publisher::new(metastore.clone());
    let publisher_handler = publisher.spawn(kill_switch.clone());
    let uploader = Uploader::new(
        metastore,
        index_storage,
        publisher_handler.mailbox().clone(),
    );
    let uploader_handler = uploader.spawn(kill_switch.clone());
    let packager = Packager::new(uploader_handler.mailbox().clone());
    let packager_handler = packager.spawn(kill_switch.clone());
    let indexer = Indexer::try_new(
        index_id,
        index_metadata.index_config.into(),
        None,
        CommitPolicy::default(),
        packager_handler.mailbox().clone(),
    )?;
    let indexer_handler = indexer.spawn(kill_switch.clone());

    // TODO the source is hardcoded here.
    let source = FileSource::try_new(
        Path::new("data/test_corpus.json"),
        indexer_handler.mailbox().clone(),
    )
    .await?;

    let _source = source.spawn(kill_switch.clone());
    Ok(())
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;
    use std::time::Duration;

    use super::spawn_indexing_pipeline;
    use quickwit_metastore::IndexMetadata;
    use quickwit_metastore::MockMetastore;

    #[tokio::test]
    async fn test_indexing_pipeline() -> anyhow::Result<()> {
        quickwit_common::setup_logging_for_tests();
        let mut metastore = MockMetastore::default();
        metastore
            .expect_index_metadata()
            .withf(|index_id| index_id == "test-index")
            .times(1)
            .returning(|_| {
                let index_metadata = IndexMetadata {
                    index_id: "test-index".to_string(),
                    index_uri: "ram://test-index".to_string(),
                    index_config: Box::new(quickwit_index_config::default_config_for_tests()),
                };
                Ok(index_metadata)
            });
        spawn_indexing_pipeline(
            "test-index".to_string(),
            Arc::new(metastore),
            Default::default(),
        )
        .await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        Ok(())
    }
}

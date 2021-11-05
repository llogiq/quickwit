// Copyright (C) 2021 Quickwit, Inc.
//
// Quickwit is offered under the AGPL v3.0 and as commercial software.
// For commercial licensing, contact us at hello@quickwit.io.
//
// AGPL:
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <http://www.gnu.org/licenses/>.

// TODO: Remove when `KinesisSource` is fully implemented.
#![allow(dead_code)]

use async_trait::async_trait;
use quickwit_actors::{Actor, ActorContext, ActorExitStatus, AsyncActor, Mailbox};
use rusoto_kinesis::{GetRecordsOutput, Kinesis, Record};
use serde_json::json;
use tokio::time::Duration;

use crate::source::kinesis::api::{get_records, get_shard_iterator};

#[derive(Debug)]
enum ShardConsumerMessage {
    /// The shard was the subject of a merge or a split and points to one child (merge) or two
    /// children (split).
    ChildShards(Vec<String>),
    Records {
        shard_id: String,
        records: Vec<Record>,
        lag_millis: Option<i64>,
    },
    /// The shard is closed after a merge or a split. There are no new records available.
    ShardClosed(String),
    /// The consumer has reached the latest record in the shard and stops if `eof_enabled` is set
    /// to true.
    ShardEOF(String),
}

#[derive(Default)]
struct ShardConsumerState {
    /// The sequence number of the last record transferred.
    current_sequence_number: Option<String>,
    /// Number of bytes transferred by the consumer.
    num_bytes_transferred: u64,
    /// Number of records transferred by the consumer.
    num_records_transferred: u64,
    next_shard_iterator: Option<String>,
}

struct ShardConsumer {
    stream_name: String,
    shard_id: String,
    /// Sequence number of the last record processed. Consumption of the shard is resumed right
    /// after this sequence number.
    from_sequence_number_exclusive: Option<String>,
    /// When this value is set to true, the consumer stops after reaching the last (most recent)
    /// record in the shard.
    eof_enabled: bool,
    state: ShardConsumerState,
    kinesis_client: Box<dyn Kinesis + Send + Sync>,
    sink: Mailbox<ShardConsumerMessage>,
}

impl ShardConsumer {
    fn new(
        stream_name: String,
        shard_id: String,
        from_sequence_number_exclusive: Option<String>,
        eof_enabled: bool,
        kinesis_client: Box<dyn Kinesis + Send + Sync>,
        sink: Mailbox<ShardConsumerMessage>,
    ) -> Self {
        Self {
            stream_name,
            shard_id,
            from_sequence_number_exclusive,
            state: Default::default(),
            eof_enabled,
            kinesis_client,
            sink,
        }
    }
}

#[derive(Debug)]
struct Loop {}

impl Actor for ShardConsumer {
    type Message = Loop;
    type ObservableState = serde_json::Value;

    fn name(&self) -> String {
        "KinesisShardConsumer".to_string()
    }

    fn observable_state(&self) -> Self::ObservableState {
        json!({
            "stream_name": self.stream_name,
            "shard_id": self.shard_id,
            "current_sequence_number": self.state.current_sequence_number,
            "num_bytes_transferred": self.state.num_bytes_transferred,
            "num_records_transferred": self.state.num_records_transferred,
        })
    }
}

#[async_trait]
impl AsyncActor for ShardConsumer {
    async fn initialize(
        &mut self,
        ctx: &ActorContext<Self::Message>,
    ) -> Result<(), ActorExitStatus> {
        self.state.next_shard_iterator = get_shard_iterator(
            &*self.kinesis_client,
            &self.stream_name,
            &self.shard_id,
            self.from_sequence_number_exclusive.clone(),
        )
        .await?;
        self.process_message(Loop {}, ctx).await?;
        Ok(())
    }

    async fn process_message(
        &mut self,
        _message: Loop,
        ctx: &ActorContext<Self::Message>,
    ) -> Result<(), ActorExitStatus> {
        if let Some(shard_iterator) = self.state.next_shard_iterator.take() {
            let response = get_records(&*self.kinesis_client, shard_iterator).await?;
            self.state.next_shard_iterator = response.next_shard_iterator;

            if !response.records.is_empty() {
                self.state.current_sequence_number = response
                    .records
                    .last()
                    .map(|record| record.sequence_number.clone());
                self.state.num_bytes_transferred += response
                    .records
                    .iter()
                    .map(|record| record.data.len() as u64)
                    .sum::<u64>();
                self.state.num_records_transferred += response.records.len() as u64;

                let message = ShardConsumerMessage::Records {
                    shard_id: self.shard_id.clone(),
                    records: response.records,
                    lag_millis: response.millis_behind_latest,
                };
                ctx.send_message(&self.sink, message).await?;
            }
            if let Some(children) = response.child_shards {
                let shard_ids: Vec<String> = children
                    .into_iter()
                    // Filter out duplicate message when two shards are merged.
                    .filter(|child| child.parent_shards.first() == Some(&self.shard_id))
                    .map(|child| child.shard_id)
                    .collect();
                if !shard_ids.is_empty() {
                    let message = ShardConsumerMessage::ChildShards(shard_ids);
                    ctx.send_message(&self.sink, message).await?;
                }
            }
            if self.eof_enabled && response.millis_behind_latest.unwrap_or(-1) == 0 {
                let message = ShardConsumerMessage::ShardEOF(self.shard_id.clone());
                ctx.send_message(&self.sink, message).await?;
                return Err(ActorExitStatus::Success);
            };
            let interval = Duration::from_millis(210);
            ctx.schedule_self_msg(interval, Loop {}).await;
            return Ok(());
        }
        let message = ShardConsumerMessage::ShardClosed(self.shard_id.clone());
        ctx.send_message(&self.sink, message).await?;
        Err(ActorExitStatus::Success)
    }
}

#[cfg(all(test, feature = "kinesis-external-service"))]
mod kinesis_localstack_tests {
    use quickwit_actors::{create_test_mailbox, Universe};

    use super::*;
    use crate::source::kinesis::api::{merge_shards, split_shard};
    use crate::source::kinesis::helpers::tests::{
        make_shard_id, put_records_into_shards, setup, teardown,
    };

    #[tokio::test]
    async fn test_shard_eof() -> anyhow::Result<()> {
        let universe = Universe::new();
        let (sink, inbox) = create_test_mailbox();
        let (kinesis_client, stream_name) = setup("test-shard-eof", 1).await?;
        let shard_id_0 = make_shard_id(0);
        let shard_consumer = ShardConsumer::new(
            stream_name.clone(),
            shard_id_0.clone(),
            None,
            true,
            Box::new(kinesis_client.clone()),
            sink.clone(),
        );
        let (_mailbox, handle) = universe.spawn_actor(shard_consumer).spawn_async();
        let (exit_status, exit_state) = handle.join().await;
        assert!(exit_status.is_success());

        let messages = inbox.drain_available_messages_for_test();
        assert_eq!(messages.len(), 1);
        assert!(matches!(
            &messages[0],
            ShardConsumerMessage::ShardEOF(shard_id) if *shard_id == shard_id_0
        ));
        let expected_state = json!({
            "stream_name": stream_name,
            "shard_id": shard_id_0,
            "current_sequence_number": serde_json::Value::Null,
            "num_bytes_transferred": 0,
            "num_records_transferred": 0,
        });
        assert_eq!(exit_state, expected_state);
        teardown(&kinesis_client, &stream_name).await;
        Ok(())
    }

    // #[tokio::test]
    // async fn test_start_at_horizon() -> anyhow::Result<()> {
    //     let (kinesis_client, stream_name) = setup("test-start-at-horizon", 1).await?;
    //     put_records_into_shards(
    //         &kinesis_client,
    //         &stream_name,
    //         [(0, "Record #00"), (0, "Record #01")],
    //     )
    //     .await?;
    //     let shard_id_0 = make_shard_id(0);
    //     let (sink, mut rx) = mpsc::channel(10);
    //     let shard_consumer = ShardConsumer::new(
    //         stream_name.clone(),
    //         shard_id_0.clone(),
    //         None,
    //         true,
    //         Box::new(kinesis_client.clone()),
    //         sink.clone(),
    //     );
    //     let shard_consumer_handle = shard_consumer.spawn();
    //     {
    //         let shard_consumer_message = recv_message(&mut rx).await?;
    //         assert!(matches!(
    //             shard_consumer_message,
    //             ShardConsumerMessage::Records { shard_id, records, lag_millis: _ } if shard_id ==
    // shard_id_0 && records.len() == 2         ));
    //     }
    //     {
    //         let shard_consumer_message = recv_message(&mut rx).await?;
    //         assert!(matches!(
    //             shard_consumer_message,
    //             ShardConsumerMessage::ShardEOF(shard_id) if shard_id == shard_id_0
    //         ));
    //     }
    //     assert!(shard_consumer_handle.await.is_ok());
    //     teardown(&kinesis_client, &stream_name).await;
    //     Ok(())
    // }

    // #[tokio::test]
    // async fn test_start_after_sequence_number() -> anyhow::Result<()> {
    //     let (kinesis_client, stream_name) = setup("test-start-after-sequence-number", 1).await?;
    //     let sequence_numbers = put_records_into_shards(
    //         &kinesis_client,
    //         &stream_name,
    //         [(0, "Record #00"), (0, "Record #01")],
    //     )
    //     .await?;
    //     let shard_id_0 = make_shard_id(0);
    //     let starting_sequence_number = sequence_numbers
    //         .get(&0)
    //         .and_then(|sequence_numbers| sequence_numbers.first());
    //     let (sink, mut rx) = mpsc::channel(10);
    //     let shard_consumer = ShardConsumer::new(
    //         stream_name.clone(),
    //         shard_id_0.clone(),
    //         starting_sequence_number.cloned(),
    //         true,
    //         Box::new(kinesis_client.clone()),
    //         sink.clone(),
    //     );
    //     let shard_consumer_handle = shard_consumer.spawn();
    //     {
    //         let shard_consumer_message = recv_message(&mut rx).await?;
    //         assert!(matches!(
    //             shard_consumer_message,
    //             ShardConsumerMessage::Records { shard_id, records, lag_millis: _ } if shard_id ==
    // shard_id_0 && records.len() == 1         ));
    //     }
    //     {
    //         let shard_consumer_message = recv_message(&mut rx).await?;
    //         assert!(matches!(
    //             shard_consumer_message,
    //             ShardConsumerMessage::ShardEOF(shard_id) if shard_id == shard_id_0
    //         ));
    //     }
    //     assert!(shard_consumer_handle.await.is_ok());
    //     teardown(&kinesis_client, &stream_name).await;
    //     Ok(())
    // }

    // // This test fails when run against the Localstack Kinesis providers `kinesis-mock` or
    // // `kinesalite`.
    // #[ignore]
    // #[tokio::test]
    // async fn test_merge_shards() -> anyhow::Result<()> {
    //     let (kinesis_client, stream_name) = setup("test-merge-shards", 2).await?;
    //     let shard_id_0 = make_shard_id(0);
    //     let shard_id_1 = make_shard_id(1);
    //     merge_shards(&kinesis_client, &stream_name, &shard_id_0, &shard_id_1).await?;

    //     let (sink, mut rx) = mpsc::channel(10);
    //     {
    //         let shard_consumer = ShardConsumer::new(
    //             stream_name.clone(),
    //             shard_id_0.clone(),
    //             None,
    //             false,
    //             Box::new(kinesis_client.clone()),
    //             sink.clone(),
    //         );
    //         let shard_consumer_handle = shard_consumer.spawn();
    //         {
    //             let shard_consumer_message = recv_message(&mut rx).await?;
    //             assert!(matches!(
    //                 shard_consumer_message,
    //                 ShardConsumerMessage::ChildShards(shard_ids) if shard_ids ==
    // vec![make_shard_id(2)]             ));
    //         }
    //         {
    //             let shard_consumer_message = recv_message(&mut rx).await?;
    //             assert!(matches!(
    //                 shard_consumer_message,
    //                 ShardConsumerMessage::ShardClosed(shard_id) if shard_id == shard_id_0
    //             ));
    //         }
    //         assert!(shard_consumer_handle.await.is_ok());
    //     }
    //     {
    //         let shard_consumer = ShardConsumer::new(
    //             stream_name.clone(),
    //             shard_id_1.clone(),
    //             None,
    //             false,
    //             Box::new(kinesis_client.clone()),
    //             sink.clone(),
    //         );
    //         let shard_consumer_handle = shard_consumer.spawn();
    //         let shard_consumer_message = recv_message(&mut rx).await?;
    //         assert!(matches!(
    //             shard_consumer_message,
    //             ShardConsumerMessage::ShardClosed(shard_id) if shard_id == shard_id_1
    //         ));
    //         assert!(shard_consumer_handle.await.is_ok());
    //     }
    //     teardown(&kinesis_client, &stream_name).await;
    //     Ok(())
    // }

    // // This test fails when run against the Localstack Kinesis providers `kinesis-mock` or
    // // `kinesalite`.
    // #[tokio::test]
    // async fn test_split_shard() -> anyhow::Result<()> {
    //     let (kinesis_client, stream_name) = setup("test-split-shard", 1).await?;
    //     let shard_id_0 = make_shard_id(0);
    //     split_shard(&kinesis_client, &stream_name, &shard_id_0, "42").await?;

    //     let (sink, mut rx) = mpsc::channel(10);
    //     let shard_consumer = ShardConsumer::new(
    //         stream_name.clone(),
    //         shard_id_0.clone(),
    //         None,
    //         false,
    //         Box::new(kinesis_client.clone()),
    //         sink.clone(),
    //     );
    //     let shard_consumer_handle = shard_consumer.spawn();
    //     {
    //         let shard_consumer_message = recv_message(&mut rx).await?;
    //         assert!(matches!(
    //             shard_consumer_message,
    //             ShardConsumerMessage::ChildShards(shard_ids) if shard_ids ==
    // vec![make_shard_id(1), make_shard_id(2)]         ));
    //     }
    //     {
    //         let shard_consumer_message = recv_message(&mut rx).await?;
    //         assert!(matches!(
    //             shard_consumer_message,
    //             ShardConsumerMessage::ShardClosed(shard_id) if shard_id == shard_id_0
    //         ));
    //     }
    //     assert!(shard_consumer_handle.await.is_ok());
    //     teardown(&kinesis_client, &stream_name).await;
    //     Ok(())
    // }
}

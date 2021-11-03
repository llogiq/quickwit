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

use anyhow::{bail, Context};
use byte_unit::Byte;
use serde::{Deserialize, Serialize};
use toml;

use crate::IndexConfig;

#[derive(Serialize, Deserialize)]
pub struct IndexingResources {
    pub num_threads: usize,
    pub heap_size: Byte,
}

// Is here a good centralized place to define those objects?
#[derive(Serialize, Deserialize)]
pub struct CommitPolicy {}

#[derive(Serialize, Deserialize)]
pub struct DemuxPolicy {}

#[derive(Serialize, Deserialize)]
pub struct MergePolicy {}

#[derive(Serialize, Deserialize)]
pub struct RetentionPolicy {}

#[derive(Serialize, Deserialize)]
pub struct IndexingSettings {
    pub commit_policy: CommitPolicy,
    pub demux_policy: DemuxPolicy,
    pub merge_policy: MergePolicy,
    pub retention_policy: RetentionPolicy,
    pub resources: IndexingResources,
}

#[derive(Serialize, Deserialize)]
pub struct SourceConfig {
    pub source_id: String,
    pub source_type: String,
    pub params: toml::Value,
}

#[derive(Serialize, Deserialize)]
pub struct IndexObject {
    pub index_id: String,
    pub index_uri: String,
    // pub metastore_uri: String,
    // pub index_mapping: Box<dyn IndexConfig>,
    pub indexing_settings: IndexingSettings,
    pub sources: Vec<SourceConfig>,
}

impl IndexObject {
    pub fn from_json(bytes: &[u8]) -> anyhow::Result<Self> {
        serde_json::from_slice::<IndexObject>(bytes).context("Failed to parse JSON index config.")
    }

    pub fn from_toml(bytes: &[u8]) -> anyhow::Result<Self> {
        toml::from_slice::<IndexObject>(bytes).context("Failed to parse TOML index config.")
    }

    pub fn from_yaml(bytes: &[u8]) -> anyhow::Result<Self> {
        serde_yaml::from_slice::<IndexObject>(bytes).context("Failed to parse YAML index config.")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn get_resource_path(relative_resource_path: &str) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("resources/tests/index_object/");
        path.push(relative_resource_path);
        path
    }

    // TODO: macrofy.
    #[test]
    fn test_parse_json() -> anyhow::Result<()> {
        let index_config_filepath = get_resource_path("hdfs-logs.json");
        let index_config_str = std::fs::read_to_string(index_config_filepath)?;
        let index_config = IndexObject::from_json(index_config_str.as_bytes())?;
        Ok(())
    }

    #[test]
    fn test_parse_toml() -> anyhow::Result<()> {
        let index_config_filepath = get_resource_path("hdfs-logs.toml");
        let index_config_str = std::fs::read_to_string(index_config_filepath)?;
        let index_config = IndexObject::from_toml(index_config_str.as_bytes())?;
        assert_eq!(index_config.index_id, "hdfs-logs");
        assert_eq!(index_config.index_uri, "s3://quickwit-indexes/hdfs-logs");
        assert_eq!(index_config.indexing_settings.resources.num_threads, 1);
        assert_eq!(
            index_config.indexing_settings.resources.heap_size,
            Byte::from_bytes(1_000_000_000)
        );
        assert_eq!(index_config.sources.len(), 2);
        Ok(())
    }

    #[test]
    fn test_parse_yaml() -> anyhow::Result<()> {
        let index_config_filepath = get_resource_path("hdfs-logs.yaml");
        let index_config_str = std::fs::read_to_string(index_config_filepath)?;
        let index_config = IndexObject::from_yaml(index_config_str.as_bytes())?;
        Ok(())
    }
}

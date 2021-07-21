use std::sync::Arc;

use quickwit_actors::Mailbox;
use quickwit_metastore::Checkpoint;

use crate::models::Batch;
use crate::models::SourceId;
use crate::sources::SourceParams;

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

pub trait Source {
    fn spawn(&self) -> anyhow::Result<()>;
}

pub async fn build_source(
    source_id: &SourceId,
    source_params: &SourceParams,
    checkpoint: &Checkpoint,
    writer_mailbox: Mailbox<Batch>,
) -> anyhow::Result<Arc<dyn Source>> {
    todo!();
}
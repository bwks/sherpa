//! Reading the recorder's consumer, `_record`, for the host's record: behind the `record`
//! feature, with the record itself.

use std::io::Write;

use anyhow::Result;
use tracing::instrument;

use super::{Channel, Delivery, Inbox, Position, RECORDER, position_of};

impl Channel {
    /// Open the recorder's consumer, with its reader's notices written to `notices`, and say how
    /// many messages wait in it, unread or unacknowledged. Like an inbox, it takes one reader at
    /// a time.
    ///
    /// # Errors
    ///
    /// Fails when there is no recorder consumer (the bus was started by an older agent-bus) or
    /// another reader is attached.
    #[instrument(skip_all)]
    pub async fn recorder<W: Write>(&self, notices: W) -> Result<(Inbox<W>, u64)> {
        self.reader(
            RECORDER,
            &format!(
                "Unable to open the recorder's consumer '{RECORDER}'. `agent-bus bus start` \
                 creates it; run it again if the bus was started by an older agent-bus"
            ),
            notices,
        )
        .await
    }
}

impl Delivery {
    /// Where the message is in the stream, from its delivery's metadata.
    ///
    /// # Errors
    ///
    /// Fails when the delivery has no `JetStream` metadata.
    pub fn position(&self) -> Result<Position> {
        position_of(&self.raw)
    }
}

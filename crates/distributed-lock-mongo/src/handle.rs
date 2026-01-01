use distributed_lock_core::{error::LockResult, traits::LockHandle};
use mongodb::{Collection, bson::doc};
use tokio::sync::watch;

use crate::document::MongoLockDocument;

pub struct MongoLockHandle {
    pub(crate) collection: Collection<MongoLockDocument>,
    pub(crate) name: String,
    pub(crate) lock_id: String,
    pub(crate) lost_token: watch::Receiver<bool>,
}

impl LockHandle for MongoLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_token
    }

    async fn release(self) -> LockResult<()> {
        // Release: delete document if _id == name AND lockId == lock_id
        // In some cases we might just want to unset fields, but deleting is cleaner if we assume 1 lock = 1 doc.
        // C# implementation seems to use the same document for the named lock, so maybe we should just clear fields?
        // Let's check the C# implementation again or stick to "delete if matches".
        // Actually, if we delete it, the next acquire will upsert it.
        // But if there are other fields we want to preserve (which there aren't in MongoLockDocument), we might want to update.
        // Given MongoLockDocument only contains lock info, deleting is probably fine and safest.

        let filter = doc! {
            "_id": &self.name,
            "lockId": &self.lock_id
        };

        // We can just delete it.
        // If we want to be "strictly" compatible with C# logic which might leave the document but "expired",
        // we could just let it expire. But explicit release usually means "make it available now".
        // Let's delete it.

        self.collection
            .delete_one(filter)
            .await
            .map_err(|e| distributed_lock_core::error::LockError::Connection(Box::new(e)))?;

        Ok(())
    }
}

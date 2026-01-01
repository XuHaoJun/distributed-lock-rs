use std::time::Duration;

use distributed_lock_core::{
    error::{LockError, LockResult},
    traits::DistributedLock,
};
use mongodb::{
    Collection, Database,
    bson::{DateTime, doc},
    options::ReturnDocument,
};
use tokio::sync::watch;
use uuid::Uuid;

use crate::{
    document::MongoLockDocument, handle::MongoLockHandle, options::MongoDistributedLockOptions,
};

pub struct MongoDistributedLock {
    name: String,
    collection: Collection<MongoLockDocument>,
    options: MongoDistributedLockOptions,
}

impl MongoDistributedLock {
    pub fn new(
        name: String,
        database: Database,
        collection_name: Option<String>,
        options: Option<MongoDistributedLockOptions>,
    ) -> Self {
        let collection_name = collection_name.as_deref().unwrap_or("DistributedLocks");
        let collection = database.collection(collection_name);
        Self {
            name,
            collection,
            options: options.unwrap_or_default(),
        }
    }

    async fn try_acquire_internal(&self) -> LockResult<Option<MongoLockHandle>> {
        let lock_id = Uuid::new_v4().to_string();
        let expiry_ms = self.options.expiry.as_millis() as i64;

        // MongoDB Pipeline construction
        // expired := ifNull(expiresAt, epoch) <= $$NOW
        let epoch = DateTime::from_millis(0);

        let expired_or_missing = doc! {
            "$lte": [
                { "$ifNull": ["$expiresAt", epoch] },
                "$$NOW"
            ]
        };

        let new_expires_at = doc! {
            "$dateAdd": {
                "startDate": "$$NOW",
                "unit": "millisecond",
                "amount": expiry_ms
            }
        };

        let set_stage = doc! {
            "$set": {
                "lockId": {
                    "$cond": [&expired_or_missing, &lock_id, "$lockId"]
                },
                "expiresAt": {
                    "$cond": [&expired_or_missing, &new_expires_at, "$expiresAt"]
                },
                "acquiredAt": {
                    "$cond": [&expired_or_missing, "$$NOW", "$acquiredAt"]
                }
            }
        };

        let pipeline = vec![set_stage];

        let filter = doc! { "_id": &self.name };

        // Use builder pattern for options in mongodb v3
        let result = self
            .collection
            .find_one_and_update(filter, pipeline)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        if let Some(doc) = result
            && doc.lock_id == lock_id
        {
            let (_tx, rx) = watch::channel(false);
            // Success!
            return Ok(Some(MongoLockHandle {
                collection: self.collection.clone(),
                name: self.name.clone(),
                lock_id,
                lost_token: rx,
            }));
        }

        Ok(None)
    }
}

impl DistributedLock for MongoDistributedLock {
    type Handle = MongoLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    async fn acquire(&self, timeout: Option<Duration>) -> LockResult<Self::Handle> {
        let start = std::time::Instant::now();
        let timeout = timeout.unwrap_or(Duration::from_secs(u64::MAX)); // Infinite-ish

        loop {
            if let Some(handle) = self.try_acquire_internal().await? {
                return Ok(handle);
            }

            if start.elapsed() >= timeout {
                return Err(LockError::Timeout(timeout));
            }

            // Simple busy wait with randomization could be better, but fixed for now
            // C# uses randomized backoff
            let sleep_time = self.options.min_busy_wait_sleep_time;
            // We could implement exponential backoff here up to max_busy_wait
            tokio::time::sleep(sleep_time).await;
        }
    }

    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        self.try_acquire_internal().await
    }
}

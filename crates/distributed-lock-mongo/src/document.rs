use mongodb::bson::DateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct MongoLockDocument {
    #[serde(rename = "_id")]
    pub id: String,

    #[serde(rename = "lockId")]
    pub lock_id: String,

    #[serde(rename = "acquiredAt")]
    pub acquired_at: DateTime,

    #[serde(rename = "expiresAt")]
    pub expires_at: DateTime,
}

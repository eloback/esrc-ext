use esrc::{
    version::{DeserializeVersion, SerializeVersion},
    Event,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(
    Event, Serialize, Deserialize, PartialEq, Debug, Clone, SerializeVersion, DeserializeVersion,
)]
#[esrc(event(name = "User"))]
pub enum Events {
    UserCreated {
        user_id: Uuid,
        name: String,
        email: String,
    },
    UserUpdated {
        user_id: Uuid,
        name: Option<String>,
        email: Option<String>,
    },
}

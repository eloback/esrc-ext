use esrc::Aggregate;

use crate::domain::user::Events;

use super::*;

#[derive(Default)]
pub struct User {
    pub id: uuid::Uuid,
    pub name: String,
    pub email: String,
}

impl Aggregate for User {
    type Command = CreateUser;
    type Event = Events;
    type Error = Errors;

    fn process(&self, command: Self::Command) -> Result<Self::Event, Self::Error> {
        let CreateUser {
            user_id,
            name,
            email,
        } = command;

        if self.id == user_id {
            return Err(Errors::UserAlreadyExists);
        }

        Ok(Self::Event::UserCreated {
            user_id,
            name,
            email,
        })
    }

    fn apply(mut self, event: &Self::Event) -> Self {
        if let Events::UserCreated {
            user_id,
            name,
            email,
        } = event
        {
            self.id = *user_id;
            self.name = name.clone();
            self.email = email.clone();
        }

        self
    }
}

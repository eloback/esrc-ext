use esrc::{
    event::event_model::{Automation, ViewAutomation},
    nats::NatsStore,
};

pub struct Feature<'a> {
    store: &'a NatsStore,
}

impl<'a> Feature<'a> {
    pub fn new(store: &'a NatsStore) -> Self {
        Self { store }
    }

    pub fn start_automation<A>(&self, project: A, feature_name: &'static str)
    where
        A: esrc::project::Project + 'static,
    {
        let store = self.store.clone();
        store.get_task_tracker().spawn(async move {
            store
                .start_automation(project, feature_name)
                .await
                .expect("automation should be able to start");
        });
    }

    pub fn start_translation<A>(
        &self,
        external_store: &NatsStore,
        project: A,
        feature_name: &'static str,
    ) where
        A: esrc::project::Project + 'static,
    {
        let store = external_store.clone();
        store.get_task_tracker().spawn(async move {
            store
                .start_automation(project, feature_name)
                .await
                .expect("automation should be able to start");
        });
    }

    pub fn start_read_model_automation<A>(&self, project: A, feature_name: &'static str)
    where
        A: esrc::project::Project + 'static,
    {
        let store = self.store.clone();
        store.get_task_tracker().spawn(async move {
            store
                .start_view_automation(project, feature_name)
                .await
                .expect("automation should be able to start");
        });
    }
}

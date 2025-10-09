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
    pub fn start_dead_letter_automation<A>(
        &self,
        durable_name: &'static str,
        stream_name: &'static str,
        feature_name: &'static str,
        dead_letter_store: A,
    ) where
        A: esrc::nats::DeadLetterStore + Clone + 'static,
    {
        let store = self.store.clone();
        let dead_letter_store = dead_letter_store.clone();

        store.get_task_tracker().spawn(async move {
            // Start dead letter automation for a specific stream and consumer
            store
                .run_dead_letter_automation(
                    dead_letter_store,
                    durable_name,
                    stream_name,
                    feature_name,
                )
                .await
                .expect("dead letter automation should be able to start");
        });
    }
}

impl<'a> Feature<'a> {
    pub fn start_legacy_automation<A>(
        &self,
        project: A,
        feature_name: &'static str,
        subjects: Vec<&'static str>,
    ) where
        A: esrc::nats::legacy::LegacyProject + 'static,
    {
        let store = self.store.clone();
        store.get_task_tracker().spawn(async move {
            store
                .run_legacy_project(project, feature_name, subjects)
                .await
                .expect("automation should be able to start");
        });
    }
}

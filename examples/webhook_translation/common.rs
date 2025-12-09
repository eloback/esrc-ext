#[derive(Clone)]
pub struct AppState {
    pub nats_client: async_nats::Client,
}

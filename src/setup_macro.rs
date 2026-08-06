//! Generic setup macros for reducing boilerplate in application initialization.
//!
//! These macros provide a consistent pattern for setting up features, read models,
//! and automations across different projects.

use std::error::Error;

/// Default number of messages processed concurrently by automations and translations.
pub const DEFAULT_MAX_CONCURRENCY: usize = 100;

/// Adds the failing component's name to errors raised while initializing application slices.
#[derive(Debug, thiserror::Error)]
#[error("failed to initialize {component}: {source}")]
pub struct SetupError {
    component: &'static str,
    #[source]
    source: Box<dyn Error + Send + Sync>,
}

impl SetupError {
    #[doc(hidden)]
    pub fn new(component: &'static str, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            component,
            source: Box::new(source),
        }
    }

    /// The slice or store that failed to initialize.
    pub fn component(&self) -> &'static str {
        self.component
    }
}

/// Setup multiple event-sourced slices with automatic lifecycle management.
///
/// This macro provides a declarative way to configure and initialize event-sourced slices
/// following Event Modeling patterns. It supports different slice types including features,
/// read models, automations, and translations.
///
/// ## Key Behavior
///
/// The macro follows a two-phase initialization pattern:
/// 1. **Setup Phase**: All `setup()` functions are called immediately in declaration order
/// 2. **Start Phase**: Background processes (automations, translations, read models) are started
///    after all setups complete
///
/// This ensures proper initialization order where all slices are configured before any
/// background event processing begins.
///
/// ## Supported Slice Types
///
/// - `Feature`: User-facing commands and features (no background processes)
/// - `Translation`: Event translation between bounded contexts
/// - `Automation`: Background event-driven processes
/// - `ReadModelRepository`: Event-sourced read model repositories
/// - `PgViewProjector`: PostgreSQL-based read model projectors with initial setup
/// - `LiveProjection`: Live projections that update in real-time
/// - `Query`: Query handlers for Inter-Domain Queries (IDQ)
///
/// ## Syntax
///
/// ```ignore
/// setup_slices! {
///     slices: {
///         slice_module_path => (SliceType, configuration),
///         another_slice => (SliceType, configuration),
///         // ... more slices
///     }
/// }
/// ```
///
/// ## Configuration by Type
///
/// ### Feature
/// ```ignore
/// my_feature::command_slice => (Feature, setup_params: { command_bus, query_bus })
/// ```
/// - Registers command handlers immediately
/// - No background processes started
///
/// ### Translation
/// ```ignore
/// integration::external_events => (Translation,
///     external_store: external,
///     max_concurrency: 50, // Optional; defaults to 100
///     setup_params: { command_bus }
/// )
/// ```
/// - Translates events from external bounded contexts
/// - Starts background subscription after setup
///
/// ### Automation
/// ```ignore
/// notifications::automation => (Automation,
///     project_start_event_store: operacoes,
///     max_concurrency: 50, // Optional; defaults to 100
///     setup_params: { command_bus }
/// )
/// ```
/// - Subscribes to domain events and executes business logic
/// - Starts background event processing after setup
///
/// ### ReadModelRepository
/// ```ignore
/// views::user_repository => (ReadModelRepository,
///     project_start_event_store: operacoes,
///     projector_version: 1, // Optional; defaults to 1
///     setup_params: { view_db }
/// )
/// ```
/// - Maintains read models from event streams
/// - Starts background projection after setup
///
/// ### PgViewProjector
/// ```ignore
/// views::customer_projection => (PgViewProjector,
///     project_start_event_store: operacoes,
///     projector_version: 1, // Optional; defaults to 1
///     setup_params: { view_db }
/// )
/// ```
/// - PostgreSQL-based projections with schema migration
/// - Awaits initial setup, then starts background projection
///
/// ### LiveProjection
/// ```ignore
/// cache::live_projection => (LiveProjection, setup_params: { cache, event_bus })
/// ```
/// - Real-time projections that update immediately
/// - No background processes (updates happen inline)
///
/// ### Query
/// ```ignore
/// analytics::query_slice => (Query, setup_params: { query_bus })
/// ```
/// - Registers query handlers immediately
/// - No background processes started
/// - Mostly used for Inter-Domain Queries (IDQ)
///
/// ## Complete Example
///
/// ```ignore
/// // First, create event stores
/// create_event_stores! {
///     operacoes => Nats {
///         context: context,
///         stream_name: "operacoes",
///         consumer_config: consumer_config,
///     },
///     external => External {
///         context: context,
///         stream_config: external_stream_config,
///     },
/// }
///
/// // Then setup slices
/// setup_slices! {
///     slices: {
///         // Features execute immediately
///         features::create_user => (Feature, setup_params: {
///             command_registry,
///             operacoes
///         }),
///         
///         // Automations start after all setups
///         automations::send_welcome_email => (Automation,
///             project_start_event_store: operacoes,
///             setup_params: { command_bus, email_service }
///         ),
///         
///         // Read models start after all setups
///         read_models::user_list => (ReadModelRepository,
///             project_start_event_store: operacoes,
///             setup_params: { view_db }
///         ),
///         
///         // Translations start after all setups
///         integration::external_events => (Translation,
///             external_store: external,
///             setup_params: { command_bus }
///         ),
///     }
/// }
/// ```
///
/// ## Module Requirements
///
/// Each slice module must provide:
/// - `setup()` function that accepts the specified parameters
/// - `FEATURE_NAME` constant (for Automation, ReadModelRepository, PgViewProjector)
/// - Return appropriate project type for background processes
///
/// ## Error Handling
///
/// - `PgViewProjector` initialization errors are wrapped in [`SetupError`] with the slice name
///   and propagated to the caller.
/// - Configurations containing a `PgViewProjector` must be invoked from an async function whose
///   error type can accept [`SetupError`].
///
/// ## Performance Considerations
///
/// - Slice factory calls are synchronous and sequential.
/// - Background processes use deferred execution via closures
/// - `PgViewProjector` initialization is awaited without blocking the async executor.
#[macro_export]
macro_rules! setup_slices {
    (
        slices: {
            $(
                $slice_path:path => ( $feature_type:ident, $($config:tt)+ )
            ),+ $(,)?
        }
    ) => {
        {
            let mut __esrc_ext_starters: std::vec::Vec<std::boxed::Box<dyn FnOnce()>> =
                std::vec::Vec::new();

            $(
                $crate::setup_slices!(@parse
                    starters: __esrc_ext_starters,
                    slice_path: $slice_path,
                    feature_type: $feature_type,
                    config: { $($config)+ }
                );
            )+

            for __esrc_ext_start in __esrc_ext_starters {
                __esrc_ext_start();
            }
        }
    };

    (@parse
        starters: $_starters:ident,
        slice_path: $slice_path:path,
        feature_type: Feature,
        config: { setup_params: { $($params:expr),* $(,)? } }
    ) => {
        {
            use $slice_path as __EsrcExtCurrentSlice;
            __EsrcExtCurrentSlice::setup($($params),*);
        }
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: Translation,
        config: {
            external_store: $external_store:expr,
            max_concurrency: $max_concurrency:expr,
            setup_params: { $($params:expr),* $(,)? }
        }
    ) => {
        {
            use $slice_path as __EsrcExtCurrentSlice;
            let __esrc_ext_project = __EsrcExtCurrentSlice::setup($($params),*);
            let __esrc_ext_store = ($external_store).clone();
            let __esrc_ext_max_concurrency = $max_concurrency;
            $starters.push(std::boxed::Box::new(move || {
                $crate::slice_runner::start_translation(
                    &__esrc_ext_store,
                    __esrc_ext_project,
                    __esrc_ext_max_concurrency,
                );
            }));
        }
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: Translation,
        config: {
            external_store: $external_store:expr,
            setup_params: { $($params:expr),* $(,)? }
        }
    ) => {
        $crate::setup_slices!(@parse
            starters: $starters,
            slice_path: $slice_path,
            feature_type: Translation,
            config: {
                external_store: $external_store,
                max_concurrency: $crate::setup_macro::DEFAULT_MAX_CONCURRENCY,
                setup_params: { $($params),* }
            }
        )
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: Automation,
        config: {
            project_start_event_store: $store:expr,
            max_concurrency: $max_concurrency:expr,
            setup_params: { $($params:expr),* $(,)? }
        }
    ) => {
        {
            use $slice_path as __EsrcExtCurrentSlice;
            let __esrc_ext_project = __EsrcExtCurrentSlice::setup($($params),*);
            let __esrc_ext_store = ($store).clone();
            let __esrc_ext_feature_name = __EsrcExtCurrentSlice::FEATURE_NAME;
            let __esrc_ext_max_concurrency = $max_concurrency;
            $starters.push(std::boxed::Box::new(move || {
                $crate::slice_runner::start_automation(
                    &__esrc_ext_store,
                    __esrc_ext_project,
                    __esrc_ext_feature_name,
                    __esrc_ext_max_concurrency,
                );
            }));
        }
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: Automation,
        config: {
            project_start_event_store: $store:expr,
            setup_params: { $($params:expr),* $(,)? }
        }
    ) => {
        $crate::setup_slices!(@parse
            starters: $starters,
            slice_path: $slice_path,
            feature_type: Automation,
            config: {
                project_start_event_store: $store,
                max_concurrency: $crate::setup_macro::DEFAULT_MAX_CONCURRENCY,
                setup_params: { $($params),* }
            }
        )
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: ReadModelRepository,
        config: {
            project_start_event_store: $store:expr,
            projector_version: $projector_version:expr,
            setup_params: { $($params:expr),* $(,)? }
        }
    ) => {
        {
            use $slice_path as __EsrcExtCurrentSlice;
            let __esrc_ext_project = __EsrcExtCurrentSlice::setup($($params),*);
            let __esrc_ext_store = ($store).clone();
            let __esrc_ext_feature_name = __EsrcExtCurrentSlice::FEATURE_NAME;
            let __esrc_ext_projector_version = $projector_version;
            $starters.push(std::boxed::Box::new(move || {
                $crate::slice_runner::start_read_model_automation_with_version(
                    &__esrc_ext_store,
                    __esrc_ext_project,
                    __esrc_ext_feature_name,
                    __esrc_ext_projector_version,
                );
            }));
        }
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: ReadModelRepository,
        config: {
            project_start_event_store: $store:expr,
            setup_params: { $($params:expr),* $(,)? }
        }
    ) => {
        $crate::setup_slices!(@parse
            starters: $starters,
            slice_path: $slice_path,
            feature_type: ReadModelRepository,
            config: {
                project_start_event_store: $store,
                projector_version: $crate::slice_runner::DEFAULT_READ_MODEL_PROJECTOR_VERSION,
                setup_params: { $($params),* }
            }
        )
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: PgViewProjector,
        config: {
            project_start_event_store: $store:expr,
            projector_version: $projector_version:expr,
            setup_params: { $($params:expr),* $(,)? }
        }
    ) => {
        {
            use $slice_path as __EsrcExtCurrentSlice;
            let __esrc_ext_project = __EsrcExtCurrentSlice::setup($($params),*);
            __esrc_ext_project
                .clone()
                .setup()
                .await
                .map_err(|__esrc_ext_error| {
                    $crate::setup_macro::SetupError::new(
                        concat!("PgViewProjector `", stringify!($slice_path), "`"),
                        __esrc_ext_error,
                    )
                })?;
            let __esrc_ext_store = ($store).clone();
            let __esrc_ext_feature_name = __EsrcExtCurrentSlice::FEATURE_NAME;
            let __esrc_ext_projector_version = $projector_version;
            $starters.push(std::boxed::Box::new(move || {
                $crate::slice_runner::start_read_model_automation_with_version(
                    &__esrc_ext_store,
                    __esrc_ext_project,
                    __esrc_ext_feature_name,
                    __esrc_ext_projector_version,
                );
            }));
        }
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: PgViewProjector,
        config: {
            project_start_event_store: $store:expr,
            setup_params: { $($params:expr),* $(,)? }
        }
    ) => {
        $crate::setup_slices!(@parse
            starters: $starters,
            slice_path: $slice_path,
            feature_type: PgViewProjector,
            config: {
                project_start_event_store: $store,
                projector_version: $crate::slice_runner::DEFAULT_READ_MODEL_PROJECTOR_VERSION,
                setup_params: { $($params),* }
            }
        )
    };

    (@parse
        starters: $_starters:ident,
        slice_path: $slice_path:path,
        feature_type: LiveProjection,
        config: { setup_params: { $($params:expr),* $(,)? } }
    ) => {
        {
            use $slice_path as __EsrcExtCurrentSlice;
            __EsrcExtCurrentSlice::setup($($params),*);
        }
    };

    (@parse
        starters: $_starters:ident,
        slice_path: $slice_path:path,
        feature_type: Query,
        config: { setup_params: { $($params:expr),* $(,)? } }
    ) => {
        {
            use $slice_path as __EsrcExtCurrentSlice;
            __EsrcExtCurrentSlice::setup($($params),*);
        }
    };

    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: $feature_type:ident,
        config: { $($config:tt)* }
    ) => {
        compile_error!(concat!(
            "invalid setup_slices! configuration for `",
            stringify!($slice_path),
            "` (slice type `",
            stringify!($feature_type),
            "`)"
        ));
    };
}

/// Create event stores with consistent configuration.
///
/// This macro supports three types of stores:
/// - `Nats`: Standard NATS event store
/// - `DeadLetter`: Dead letter queue store for failed events
/// - `External`: External event store for cross-context communication
///
/// # Structured syntax
///
/// Store configuration uses named fields so new options can be added without extending a
/// positional tuple. `NatsStoreOptions::default()` requests one replica (R1), while
/// `NatsStoreOptions::replicated()` requests three replicas (R3).
///
/// ```ignore
/// create_event_stores! {
///     operations => Nats {
///         context: jetstream_context,
///         stream_name: "operations",
///         consumer_config: consumer_config,
///     },
///     replicated_operations => Nats {
///         context: jetstream_context,
///         stream_name: "replicated_operations",
///         options: esrc::prelude::NatsStoreOptions::replicated(),
///         consumer_config: consumer_config,
///     },
///     dead_letters => DeadLetter {
///         context: dead_letter_context,
///         stream_name: "dead_letters",
///     },
///     external => External {
///         context: external_context,
///         stream_config: external_stream_config,
///     },
/// }
/// ```
///
/// Each store will be created as a variable with its concrete type:
/// - `operations` will have type `esrc::prelude::NatsStore`
/// - `dead_letters` will have type `nats_dead_letter::NatsStore`
/// - `external` will have type `esrc_ext::translation::ExternalStore`
///
/// # Parameters
/// - `store_name`: Variable name for the store
/// - `store_type`: Either `Nats`, `DeadLetter`, or `External`
/// - `context`: JetStream context for the store
/// - `stream_name`: Name of the NATS stream
/// - `options`: Optional [`esrc::prelude::NatsStoreOptions`]; defaults to R1
/// - `consumer_config`: Consumer configuration of type `async_nats::jetstream::consumer::pull::Config` for NATS
/// - `stream_config`: Stream configuration of type `async_nats::jetstream::stream::Config` for External stores
#[macro_export]
macro_rules! create_event_stores {
    () => {};

    // Parse named store entries recursively so every generated binding remains in the caller's
    // scope.
    (
        $store_name:ident => $store_type:ident { $($config:tt)* }
        $(, $($rest:tt)*)?
    ) => {
        $crate::create_event_stores!(@single
            name: $store_name,
            store_type: $store_type,
            config: { $($config)* }
        );
        $(
            $crate::create_event_stores!($($rest)*);
        )?
    };

    // NATS with explicit options. Passing the options object through makes this arm compatible
    // with future fields added by esrc without teaching this macro about each field.
    (@single
        name: $store_name:ident,
        store_type: Nats,
        config: {
            context: $context:expr,
            stream_name: $stream_name:expr,
            options: $options:expr,
            consumer_config: $consumer_config:expr $(,)?
        }
    ) => {
        let $store_name = esrc::prelude::NatsStore::try_new_with_options(
            ($context).clone(),
            $stream_name,
            $options,
        )
        .await?
        .update_durable_consumer_option($consumer_config);
    };

    // Omitting options selects the default R1 behavior.
    (@single
        name: $store_name:ident,
        store_type: Nats,
        config: {
            context: $context:expr,
            stream_name: $stream_name:expr,
            consumer_config: $consumer_config:expr $(,)?
        }
    ) => {
        $crate::create_event_stores!(@single
            name: $store_name,
            store_type: Nats,
            config: {
                context: $context,
                stream_name: $stream_name,
                options: esrc::prelude::NatsStoreOptions::default(),
                consumer_config: $consumer_config,
            }
        );
    };

    (@single
        name: $store_name:ident,
        store_type: DeadLetter,
        config: {
            context: $context:expr,
            stream_name: $stream_name:expr $(,)?
        }
    ) => {
        let $store_name = nats_dead_letter::NatsStore::try_new(
            ($context).clone(),
            $stream_name,
        )
        .await?;
    };

    (@single
        name: $store_name:ident,
        store_type: External,
        config: {
            context: $context:expr,
            stream_config: $stream_config:expr $(,)?
        }
    ) => {
        let __esrc_ext_stream_config = $stream_config;
        let $store_name = $crate::translation::ExternalStore::try_new(
            ($context).clone(),
            &__esrc_ext_stream_config,
        )
        .await?;
    };

    (@single
        name: $store_name:ident,
        store_type: $store_type:ident,
        config: { $($config:tt)* }
    ) => {
        compile_error!(concat!(
            "invalid create_event_stores! configuration for `",
            stringify!($store_name),
            "` (store type `",
            stringify!($store_type),
            "`)"
        ));
    };

}

/// Create a registry and bus pair (command or query).
///
/// # Syntax
/// ```ignore
/// let (command_registry, command_bus) = create_registry_and_bus!(Command);
/// let (query_registry, query_bus) = create_registry_and_bus!(Query);
/// ```
#[macro_export]
macro_rules! create_registry_and_bus {
    (Command) => {{
        let registry = std::sync::Arc::new(std::sync::Mutex::new(
            discern::registry::CommandHandlerRegistry::new(),
        ));
        let bus: std::sync::LazyLock<
            discern::command::CommandBus,
            Box<dyn FnOnce() -> discern::command::CommandBus + Send>,
        > = {
            let registry_clone = registry.clone();
            std::sync::LazyLock::new(Box::new(move || {
                let mut registry_guard = registry_clone.lock().unwrap();
                let old_registry = std::mem::replace(
                    &mut *registry_guard,
                    discern::registry::CommandHandlerRegistry::new(),
                );
                discern::command::CommandBus::new(old_registry)
            }))
        };
        (registry, std::sync::Arc::new(bus))
    }};

    (Query) => {{
        let registry = std::sync::Arc::new(std::sync::Mutex::new(
            discern::registry::QueryHandlerRegistry::new(),
        ));
        let bus: std::sync::LazyLock<
            discern::query::QueryBus,
            Box<dyn FnOnce() -> discern::query::QueryBus + Send>,
        > = {
            let registry_clone = registry.clone();
            std::sync::LazyLock::new(Box::new(move || {
                let mut registry_guard = registry_clone.lock().unwrap();
                let old_registry = std::mem::replace(
                    &mut *registry_guard,
                    discern::registry::QueryHandlerRegistry::new(),
                );
                discern::query::QueryBus::new(old_registry)
            }))
        };
        (registry, std::sync::Arc::new(bus))
    }};
}
#[cfg(test)]
mod tests {
    use std::error::Error as _;

    mod first_slice {
        pub fn setup(order: &mut Vec<&'static str>) {
            order.push("first");
        }
    }

    mod second_slice {
        pub fn setup(order: &mut Vec<&'static str>) {
            order.push("second");
        }
    }

    mod parameterless_slice {
        pub fn setup() {}
    }

    // Compile-time coverage for the public store syntax. The function is never executed because
    // constructing the stores requires a live NATS server.
    #[allow(dead_code)]
    async fn structured_event_store_configuration_compiles(
        context: esrc::prelude::async_nats::jetstream::Context,
        consumer_config: esrc::prelude::async_nats::jetstream::consumer::pull::Config,
        external_stream_config: esrc::prelude::async_nats::jetstream::stream::Config,
    ) -> esrc::error::Result<()> {
        crate::create_event_stores! {
            r1 => Nats {
                context: context,
                stream_name: "r1",
                consumer_config: consumer_config.clone(),
            },
            r3 => Nats {
                context: context,
                stream_name: "r3",
                options: esrc::prelude::NatsStoreOptions::replicated(),
                consumer_config: consumer_config.clone(),
            },
            external => External {
                context: context,
                stream_config: external_stream_config,
            },
        }

        let _ = (r1, r3, external);
        Ok(())
    }

    #[test]
    fn setup_slices_runs_setup_in_declaration_order() {
        let mut order = Vec::new();

        crate::setup_slices! {
            slices: {
                crate::setup_macro::tests::first_slice => (
                    Feature,
                    setup_params: { &mut order }
                ),
                crate::setup_macro::tests::second_slice => (
                    Query,
                    setup_params: { &mut order }
                ),
                crate::setup_macro::tests::parameterless_slice => (
                    LiveProjection,
                    setup_params: {}
                ),
            }
        }

        assert_eq!(order, ["first", "second"]);
    }

    #[test]
    fn setup_error_preserves_context_and_source() {
        let error = super::SetupError::new(
            "PgViewProjector `users`",
            std::io::Error::other("database unavailable"),
        );

        assert_eq!(error.component(), "PgViewProjector `users`");
        assert_eq!(
            error.to_string(),
            "failed to initialize PgViewProjector `users`: database unavailable"
        );
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("database unavailable")
        );
    }
}

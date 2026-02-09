//! Generic setup macros for reducing boilerplate in application initialization.
//!
//! These macros provide a consistent pattern for setting up features, read models,
//! and automations across different projects.

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
///     setup_params: { view_db }
/// )
/// ```
/// - PostgreSQL-based projections with schema migration
/// - Runs initial setup synchronously, then starts background projection
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
///     operacoes => (Nats, context, "operacoes", consumer_config),
///     external => (External, context, external_stream_config),
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
/// - PgViewProjector panics if initial setup fails with message "Could not start PgViewProjector"
/// - Other failures propagate through the setup functions
///
/// ## Performance Considerations
///
/// - All `setup()` calls are synchronous and sequential
/// - Background processes use deferred execution via closures
/// - No async/await in macro expansion (use `futures::executor::block_on` for PgViewProjector)
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
            // Collection to store startup closures
            let mut __project_starters: Vec<Box<dyn FnOnce()>> = Vec::new();

            $(
                $crate::setup_slices!(@parse
                    starters: __project_starters,
                    slice_path: $slice_path,
                    feature_type: $feature_type,
                    config: { $($config)+ }
                );
            )+

            // Execute all project starters after setup is complete
            for starter in __project_starters {
                starter();
            }
        }
    };

    // Parse Feature configuration
    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: Feature,
        config: { setup_params: { $($params:expr),+ $(,)? } }
    ) => {
        {
            // Used to fix the compiler error about 'AST fragment opacity', and enforce that the path is a path and not an expression.
            use $slice_path as CurrentSlice;
            CurrentSlice::setup($($params),+);
        }
    };

    // Parse Translation configuration
    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: Translation,
        config: { external_store: $external_store:ident, setup_params: { $($params:expr),+ $(,)? } }
    ) => {
        {
            // Used to fix the compiler error about 'AST fragment opacity', and enforce that the path is a path and not an expression.
            use $slice_path as CurrentSlice;
            let project = CurrentSlice::setup($($params),+);
            let external_store = $external_store.clone();

            // TODO: The max_concurrency of 100 is hardcoded here
            // But ideally it should be possible to configure it via the macro parameter or via a environment variable
            $starters.push(Box::new(move || {
                esrc_ext::slice_runner::start_translation(
                    &external_store,
                    project
                    100,
                );
            }));
        }
    };

    // Parse Automation configuration
    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: Automation,
        config: { project_start_event_store: $store:ident, setup_params: { $($params:expr),+ $(,)? } }
    ) => {
        {
            // Used to fix the compiler error about 'AST fragment opacity', and enforce that the path is a path and not an expression.
            use $slice_path as CurrentSlice;
            let project = CurrentSlice::setup($($params),+);
            let store = $store.clone();
            let feature_name = CurrentSlice::FEATURE_NAME;

            // TODO: The max_concurrency of 100 is hardcoded here
            // But ideally it should be possible to configure it via the macro parameter or via a environment variable
            $starters.push(Box::new(move || {
                esrc_ext::slice_runner::start_automation(
                    &store,
                    project,
                    feature_name,
                    100,
                );
            }));
        }
    };

    // Parse ReadModelRepository configuration
    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: ReadModelRepository,
        config: { project_start_event_store: $store:ident, setup_params: { $($params:expr),+ $(,)? } }
    ) => {
        {
            // Used to fix the compiler error about 'AST fragment opacity', and enforce that the path is a path and not an expression.
            use $slice_path as CurrentSlice;
            let project = CurrentSlice::setup($($params),+);
            let store = $store.clone();
            let feature_name = CurrentSlice::FEATURE_NAME;
            $starters.push(Box::new(move || {
                esrc_ext::slice_runner::start_read_model_automation(
                    &store,
                    project,
                    feature_name
                );
            }));
        }
    };

    // Parse PgViewProjector configuration
    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: PgViewProjector,
        config: { project_start_event_store: $store:ident, setup_params: { $($params:expr),+ $(,)? } }
    ) => {
        {
            // Used to fix the compiler error about 'AST fragment opacity', and enforce that the path is a path and not an expression.
            use $slice_path as CurrentSlice;
            let project = CurrentSlice::setup($($params),+);
            futures::executor::block_on(project.clone().setup()).expect("Could not start PgViewProjector");
            let store = $store.clone();
            let feature_name = CurrentSlice::FEATURE_NAME;
            $starters.push(Box::new(move || {
                esrc_ext::slice_runner::start_read_model_automation(
                    &store,
                    project,
                    feature_name
                );
            }));
        }
    };

    // Parse LiveProjection configuration
    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: LiveProjection,
        config: { setup_params: { $($params:expr),+ $(,)? } }
    ) => {
        {
            // Used to fix the compiler error about 'AST fragment opacity', and enforce that the path is a path and not an expression.
            use $slice_path as CurrentSlice;
            CurrentSlice::setup($($params),+);
        }
    };

    // Parse Query configuration
    (@parse
        starters: $starters:ident,
        slice_path: $slice_path:path,
        feature_type: Query,
        config: { setup_params: { $($params:expr),+ $(,)? } }
    ) => {
        {
            // Used to fix the compiler error about 'AST fragment opacity', and enforce that the path is a path and not an expression.
            use $slice_path as CurrentSlice;
            CurrentSlice::setup($($params),+);
        }
    };
}

/// Create event stores with consistent configuration.
///
/// This macro supports three types of stores:
/// - `Nats`: Standard NATS event store
/// - `DeadLetter`: Dead letter queue store for failed events
/// - `External`: External event store for cross-context communication
///
/// # Syntax
/// ```ignore
/// create_event_stores! {
///     operations => (Nats, jetstream_context, "operacoes", consumer_config),
///     formalizations => (Nats, jetstream_context, "formalizations", consumer_config),
///     dead_letters => (DeadLetter, jetstream_context, "dead_letters"),
///     external => (External, external_context, "external", external_stream_config),
/// }
/// ```
///
/// Each store will be created as a variable with its concrete type:
/// - `operations` will have type `esrc::nats::NatsStore`
/// - `dead_letters` will have type `nats_dead_letter::NatsStore`
/// - `external` will have type `esrc_ext::translation::ExternalStore`
///
/// # Parameters
/// - `store_name`: Variable name for the store
/// - `store_type`: Either `Nats`, `DeadLetter`, or `External`
/// - `context`: JetStream context for the store
/// - `stream_name`: Name of the NATS stream
/// - `consumer_config`: Consumer configuration of type `async_nats::jetstream::consumer::pull::Config` for NATS
/// - `stream_config`: Stream configuration of type `async_nats::jetstream::stream::Config` for External stores
#[macro_export]
macro_rules! create_event_stores {
    (
        $(
            $store_name:ident => ( $store_type:ident, $($args:tt)+ )
        ),+ $(,)?
    ) => {
        $(
            $crate::create_event_stores!(@single
                name: $store_name,
                store_type: $store_type,
                args: ( $($args)+ )
            );
        )+
    };

    // Handler for Nats store
    (@single
        name: $store_name:ident,
        store_type: Nats,
        args: ( $context:expr, $stream_name:expr, $consumer_config:expr )
    ) => {
        let $store_name = esrc::nats::NatsStore::try_new($context.clone(), $stream_name).await?
            .update_durable_consumer_option($consumer_config);
    };

    // Handler for DeadLetter store
    (@single
        name: $store_name:ident,
        store_type: DeadLetter,
        args: ( $context:expr, $stream_name:expr )
    ) => {
        let $store_name = nats_dead_letter::NatsStore::try_new($context.clone(), $stream_name).await?;
    };

    // Handler for External store
    (@single
        name: $store_name:ident,
        store_type: External,
        args: ( $context:expr, $stream_config:expr )
    ) => {
        let $store_name = esrc_ext::translation::ExternalStore::try_new($context.clone(), $stream_config).await?;
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
    /// Note: These are compile-time tests to ensure the macros expand correctly.
    /// They won't execute but will fail to compile if the macro syntax is broken.
    #[allow(dead_code)]
    fn test_macro_compilation() {
        // This function exists only to verify macro syntax at compile time
    }
}

use std::path::Path;
use std::{net::SocketAddr, sync::Arc};

use apollo_compiler::{Name, Schema, ast::OperationType, validation::Valid};
use axum_otel_metrics::HttpMetricsLayerBuilder;
use axum_tracing_opentelemetry::middleware::OtelInResponseLayer;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServiceExt as _, transport::stdio};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::server::states::telemetry::otel_context_middleware;
use crate::tiers::{LOAD_TIER_TOOL_NAME, LoadTier, SessionStateMap};
use crate::{
    cors::CorsConfig,
    errors::ServerError,
    explorer::{EXPLORER_TOOL_NAME, Explorer},
    health::HealthCheck,
    introspection::tools::{
        execute::{EXECUTE_TOOL_NAME, Execute},
        introspect::{INTROSPECT_TOOL_NAME, Introspect},
        search::{SEARCH_TOOL_NAME, Search},
        validate::{VALIDATE_TOOL_NAME, Validate},
    },
    operations::{MutationMode, RawOperation},
    server::Transport,
};
use apollo_mcp_rhai::{RhaiEngine, checkpoints};

use super::{Config, Running, shutdown_signal};

pub(super) struct Starting {
    pub(super) config: Config,
    pub(super) schema: Valid<Schema>,
    pub(super) operations: Vec<RawOperation>,
}

impl Starting {
    pub(super) async fn start(mut self) -> Result<Running, ServerError> {
        let peers = Arc::new(RwLock::new(Vec::new()));

        let operations: Vec<_> = self
            .operations
            .into_iter()
            .filter_map(|operation| {
                operation
                    .into_operation(
                        &self.schema,
                        self.config.custom_scalar_map.as_ref(),
                        self.config.mutation_mode,
                        self.config.disable_type_description,
                        self.config.disable_schema_description,
                        self.config.enable_output_schema,
                        &self.config.annotations,
                        &self.config.descriptions,
                    )
                    .unwrap_or_else(|error| {
                        error!("Invalid operation: {}", error);
                        None
                    })
            })
            .collect();

        debug!(
            "Loaded {} operations:\n{}",
            operations.len(),
            serde_json::to_string_pretty(&operations)?
        );

        let execute_tool = self.config.execute_introspection.then(|| {
            Execute::new(
                self.config.mutation_mode,
                self.config.execute_tool_hint.as_deref(),
            )
        });

        let root_query_type = self
            .config
            .introspect_introspection
            .then(|| {
                self.schema
                    .root_operation(OperationType::Query)
                    .map(Name::as_str)
                    .map(|s| s.to_string())
            })
            .flatten();
        let root_mutation_type = self
            .config
            .introspect_introspection
            .then(|| {
                matches!(self.config.mutation_mode, MutationMode::All)
                    .then(|| {
                        self.schema
                            .root_operation(OperationType::Mutation)
                            .map(Name::as_str)
                            .map(|s| s.to_string())
                    })
                    .flatten()
            })
            .flatten();
        let apps = crate::apps::load_from_path(
            Path::new("apps"),
            &self.schema,
            self.config.custom_scalar_map.as_ref(),
            self.config.mutation_mode,
            self.config.disable_type_description,
            self.config.disable_schema_description,
            self.config.enable_output_schema,
        )
        .map_err(ServerError::Apps)?;
        let prompts =
            crate::prompts::load_from_path(Path::new("prompts")).map_err(ServerError::Prompts)?;
        let schema = Arc::new(RwLock::new(self.schema));
        let introspect_tool = self.config.introspect_introspection.then(|| {
            Introspect::new(
                schema.clone(),
                root_query_type,
                root_mutation_type,
                self.config.introspect_minify,
                self.config.introspect_tool_hint.as_deref(),
            )
        });
        let validate_tool = self
            .config
            .validate_introspection
            .then(|| Validate::new(schema.clone(), self.config.validate_tool_hint.as_deref()));
        let search_tool = if self.config.search_introspection {
            Some(Search::new(
                schema.clone(),
                matches!(self.config.mutation_mode, MutationMode::All),
                self.config.search_leaf_depth,
                self.config.index_memory_bytes,
                self.config.search_minify,
                self.config.search_tool_hint.as_deref(),
            )?)
        } else {
            None
        };

        let explorer_tool = self.config.explorer_graph_ref.map(Explorer::new);

        let cancellation_token = CancellationToken::new();

        // Create health checks only when StreamableHttp transport is enabled.
        let health_check = match (&self.config.transport, self.config.health_check.enabled) {
            (Transport::StreamableHttp { .. }, true) => {
                Some(HealthCheck::new(self.config.health_check.clone()))
            }
            _ => None, // No health checks for Stdio or when disabled.
        };

        let mut engine = RhaiEngine::new(&self.config.rhai_dir);
        engine.load_from_path().map_err(|err| {
            error!("Error loading Rhai scripts: {err}");
            ServerError::RhaiError
        })?;
        let engine = Arc::new(parking_lot::Mutex::new(engine));

        if cfg!(feature = "experimental_rhai") {
            checkpoints::on_startup(&engine).map_err(|err| {
                error!("Error when executing on_startup hook: {err}");
                ServerError::RhaiError
            })?;
        }

        // Move into `Running` so we do not clone the full string (`config.instructions` is not read afterward).
        let instructions = std::mem::take(&mut self.config.instructions);

        // Progressive tool disclosure: validate tools config against the
        // set of tools this server will actually serve, then build the
        // synthetic `load_tier` tool. When neither `bootstrap` nor
        // `tiers` are configured, filtering is skipped entirely.
        let tools_config = std::mem::take(&mut self.config.tools_config);
        let (load_tier_tool, bootstrap_tools) = if tools_config.is_enabled() {
            let mut known: std::collections::HashSet<&str> = operations
                .iter()
                .map(|op| op.as_ref().name.as_ref())
                .collect();
            if execute_tool.is_some() {
                known.insert(EXECUTE_TOOL_NAME);
            }
            if introspect_tool.is_some() {
                known.insert(INTROSPECT_TOOL_NAME);
            }
            if search_tool.is_some() {
                known.insert(SEARCH_TOOL_NAME);
            }
            if explorer_tool.is_some() {
                known.insert(EXPLORER_TOOL_NAME);
            }
            if validate_tool.is_some() {
                known.insert(VALIDATE_TOOL_NAME);
            }

            let referenced = tools_config.referenced_tool_names();
            let missing: Vec<&&str> = referenced.difference(&known).collect();
            if !missing.is_empty() {
                return Err(ServerError::ToolsConfig(format!(
                    "unknown tool name(s) in `tools.bootstrap` / `tools.tiers`: {missing:?}. \
                     Known tools: {known:?}"
                )));
            }
            if tools_config
                .referenced_tool_names()
                .contains(LOAD_TIER_TOOL_NAME)
            {
                return Err(ServerError::ToolsConfig(format!(
                    "`{LOAD_TIER_TOOL_NAME}` is reserved and cannot appear in \
                     `tools.bootstrap` or `tools.tiers`"
                )));
            }

            let bootstrap: std::collections::HashSet<String> =
                tools_config.bootstrap.iter().cloned().collect();
            let session_state: SessionStateMap =
                Arc::new(RwLock::new(std::collections::HashMap::new()));
            let load_tier = LoadTier::new(tools_config.tiers, session_state);
            (Some(load_tier), Arc::new(bootstrap))
        } else {
            (None, Arc::new(std::collections::HashSet::new()))
        };

        let running = Running {
            schema,
            operations: Arc::new(RwLock::new(operations)),
            apps,
            prompts,
            headers: self.config.headers,
            forward_headers: self.config.forward_headers.clone(),
            endpoint: self.config.endpoint,
            execute_tool,
            introspect_tool,
            search_tool,
            explorer_tool,
            validate_tool,
            load_tier_tool,
            bootstrap_tools,
            custom_scalar_map: self.config.custom_scalar_map,
            peers,
            cancellation_token: cancellation_token.clone(),
            mutation_mode: self.config.mutation_mode,
            disable_type_description: self.config.disable_type_description,
            disable_schema_description: self.config.disable_schema_description,
            enable_output_schema: self.config.enable_output_schema,
            disable_auth_token_passthrough: self.config.disable_auth_token_passthrough,
            descriptions: self.config.descriptions,
            annotations: self.config.annotations,
            health_check: health_check.clone(),
            server_info: self.config.server_info.clone(),
            instructions,
            rhai_engine: engine,
        };

        match self.config.transport {
            Transport::StreamableHttp {
                auth,
                address,
                port,
                stateful_mode,
                host_validation,
            } => {
                info!(port = ?port, address = ?address, "Starting MCP server in Streamable HTTP mode");
                let running = running.clone();
                let listen_address = SocketAddr::new(address, port);
                let http_config = host_validation.apply_to(
                    StreamableHttpServerConfig::default().with_stateful_mode(stateful_mode),
                );
                let service = StreamableHttpService::new(
                    move || Ok(running.clone()),
                    LocalSessionManager::default().into(),
                    http_config,
                );
                let mut router = axum::Router::new().nest_service("/mcp", service);
                if let Some(auth) = auth {
                    router = auth
                        .enable_middleware(router, self.config.required_scopes.clone())
                        .inspect_err(|e| {
                            error!("Failed to enable auth middleware: {}", e);
                        })?;
                }
                let mut router = with_cors(router, &self.config.cors)?
                    .layer(HttpMetricsLayerBuilder::new().build())
                    // include trace context as header into the response
                    .layer(OtelInResponseLayer)
                    // start OpenTelemetry trace on incoming request
                    .layer(axum::middleware::from_fn(otel_context_middleware));

                // Add health check endpoint if configured
                if let Some(health_check) = health_check.filter(|h| h.config().enabled) {
                    router = with_cors(health_check.enable_router(router), &self.config.cors)?;
                }

                let tcp_listener = tokio::net::TcpListener::bind(listen_address).await?;
                let shutdown_token = cancellation_token.clone();
                tokio::spawn(async move {
                    // Shut down when either a signal (CTRL+C/SIGTERM) is received
                    // or the cancellation token is cancelled (e.g., config change restart).
                    let graceful_shutdown = async move {
                        tokio::select! {
                            _ = shutdown_signal() => {},
                            _ = shutdown_token.cancelled() => {},
                        }
                    };
                    // Health check is already active from creation
                    if let Err(e) = axum::serve(tcp_listener, router)
                        .with_graceful_shutdown(graceful_shutdown)
                        .await
                    {
                        // This can never really happen
                        error!("Failed to start MCP server: {e:?}");
                    }
                });
            }
            Transport::Stdio {} => {
                info!("Starting MCP server in stdio mode");
                let service = running
                    .clone()
                    .serve(stdio())
                    .await
                    .inspect_err(|e| {
                        error!("serving error: {:?}", e);
                    })
                    .map_err(Box::new)?;
                service.waiting().await.map_err(ServerError::StartupError)?;
            }
        }

        Ok(running)
    }
}

fn with_cors(router: axum::Router, config: &CorsConfig) -> Result<axum::Router, ServerError> {
    if config.enabled {
        let cors_layer = config.build_cors_layer().inspect_err(|e| {
            error!("Failed to build CORS layer: {}", e);
        })?;
        Ok(router.layer(cors_layer))
    } else {
        Ok(router)
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderMap;
    use url::Url;

    use crate::health::HealthCheckConfig;
    use crate::host_validation::HostValidationConfig;

    use super::*;

    #[tokio::test]
    async fn start_basic_server() {
        let starting = Starting {
            config: Config {
                rhai_dir: std::path::PathBuf::from("rhai"),
                transport: Transport::StreamableHttp {
                    auth: None,
                    address: "127.0.0.1".parse().unwrap(),
                    port: 7799,
                    stateful_mode: false,
                    host_validation: HostValidationConfig::default(),
                },
                endpoint: Url::parse("http://localhost:4000").expect("valid url"),
                mutation_mode: MutationMode::All,
                execute_introspection: true,
                headers: HeaderMap::new(),
                forward_headers: vec![],
                validate_introspection: true,
                introspect_introspection: true,
                search_introspection: true,
                introspect_minify: false,
                search_minify: false,
                execute_tool_hint: None,
                introspect_tool_hint: None,
                search_tool_hint: None,
                validate_tool_hint: None,
                explorer_graph_ref: None,
                custom_scalar_map: None,
                disable_type_description: false,
                disable_schema_description: false,
                enable_output_schema: false,
                disable_auth_token_passthrough: false,
                descriptions: std::collections::HashMap::new(),
                annotations: std::collections::HashMap::new(),
                required_scopes: std::collections::HashMap::new(),
                search_leaf_depth: 5,
                index_memory_bytes: 1024 * 1024 * 1024,
                health_check: HealthCheckConfig {
                    enabled: true,
                    ..Default::default()
                },
                cors: Default::default(),
                server_info: Default::default(),
                instructions: None,
                tools_config: crate::tools_config::Tools::default(),
            },
            schema: Schema::parse_and_validate("type Query { hello: String }", "test.graphql")
                .expect("Valid schema"),
            operations: vec![],
        };
        let running = starting.start();
        assert!(running.await.is_ok());
    }
}

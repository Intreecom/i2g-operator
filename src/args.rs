#[derive(clap::Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
/// Ingress 2 gateway operator.
///
/// Automatically converts all ingresses to
/// gateway-api compatible resources.
pub struct I2GArgs {
    // Default gateway name
    #[arg(long, env = "I2G_DEFAULT_GATEWAY_NAME")]
    pub default_gateway_name: String,

    /// Default gateway's namespace if default_gateway_name is present
    #[arg(long, default_value = "default", env = "I2G_DEFAULT_GATEWAY_NAMESPACE")]
    pub default_gateway_namespace: String,

    // Whether to link created resources to the ingress via owner references and labels
    //
    // This is ueful for deleting all HTTP or TCPRoute objects when an Ingress is deleted
    #[arg(long, env = "I2G_LINK_TO_INGRESS", default_value_t = true)]
    pub link_to_ingress: bool,

    /// Whether to use experimental gateway-api resources like TCPRoutes.
    #[arg(long, env = "I2G_EXPERIMENTAL", default_value_t = false)]
    pub experimental: bool,

    /// Log level for the operator.
    #[arg(long, env = "I2G_LOG_LEVEL", default_value_t = tracing::level_filters::LevelFilter::INFO)]
    pub log_level: tracing::level_filters::LevelFilter,

    /// Whether to skip ingresses by default unless they have the annotation
    /// `i2g-operator/translate: "true"`
    #[arg(long, env = "I2G_SKIP_BY_DEFAULT", default_value_t = false)]
    pub skip_by_default: bool,
}

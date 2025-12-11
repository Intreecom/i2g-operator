/// This annotation will split ingress rules to a new HTTPSerice for each rule.
/// Of the ingress. It's usefull because HTTPRoute resource can only have up to 16
/// rules.
pub const SPLIT_ROUTES: &'static str = "i2g-operator/split-paths";
/// This annotation will mark an ingress to be translated by the operator.
/// If it's false the operator will skip the ingress in any way.
pub const TRANSLATE_INGRESS: &'static str = "i2g-operator/translate";

/// Override gateway name annotation.
pub const GATEWAY_NAME: &'static str = "i2g-operator/gateway-name";
/// Override gateway namespace annotation.
pub const GATEWAY_NAMESPACE: &'static str = "i2g-operator/gateway-namespace";

/// What section to use for resulting Routes.
pub const DESIRED_SECTION: &'static str = "i2g-operator/section-name";

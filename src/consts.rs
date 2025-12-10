/// This annotation will split ingress rules to a new HTTPSerice for each rule.
/// Of the ingress. It's usefull because HTTPRoute resource can only have up to 16
/// rules.
pub const SPLIT_ROUTES: &'static str = "i2g-gateway/split_routes";

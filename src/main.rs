use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use gateway_api::{
    apis::experimental::tcproutes::{
        TCPRoute, TCPRouteParentRefs, TCPRouteRules, TCPRouteRulesBackendRefs, TCPRouteSpec,
    },
    gateways,
    httproutes::{
        HTTPRoute, HTTPRouteParentRefs, HTTPRouteRules, HTTPRouteRulesBackendRefs,
        HTTPRouteRulesMatches, HTTPRouteRulesMatchesPath, HTTPRouteRulesMatchesPathType,
        HTTPRouteSpec,
    },
};
use k8s_openapi::api::{
    core::v1::Service,
    networking::v1::{Ingress, IngressServiceBackend, ServiceBackendPort},
};
use kube::{
    Api, Resource, ResourceExt,
    api::{ObjectMeta, PatchParams},
    runtime::controller::Action,
};
use tracing::Instrument;

use crate::{
    err::{I2GError, I2GResult},
    utils::ObjectMetaI2GExt,
    value_filters::{HeadersMatchersList, MatchRule, MatcherList, QueryMatchersList},
};

mod args;
mod consts;
mod ctx;
mod err;
mod utils;
mod value_filters;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub struct RouteInputInfo<'a> {
    pub ingress_name: String,
    pub ingress_meta: &'a ObjectMeta,
    pub ingress_namespace: String,
    pub gw_name: String,
    pub gw_namespace: String,
    pub section_name: Option<String>,
    pub hostname: String,
    pub header_matchers: Option<value_filters::HeadersMatchersList>,
    pub query_matchers: Option<value_filters::QueryMatchersList>,
}

async fn get_svc_port_number(
    api: Api<Service>,
    svc_name: &str,
    port_def: &ServiceBackendPort,
) -> Option<i32> {
    if let Some(number) = port_def.number {
        return Some(number);
    }
    let Some(port_name) = &port_def.name else {
        return None;
    };
    let Some(port) = api
        .get(svc_name)
        .await
        .ok()
        .and_then(|o| o.spec)
        .and_then(|s| s.ports)
        .and_then(|ports| {
            ports
                .into_iter()
                .find(|port| port.name.as_ref() == Some(port_name))
        })
    else {
        tracing::warn!(
            "Cannot resolve port {port_name} for service {svc_name} or service {svc_name} was not found"
        );
        return None;
    };

    return Some(port.port);
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EitherQueryOrHeaderMatcher {
    Header(MatchRule),
    Query(MatchRule),
}

impl From<EitherQueryOrHeaderMatcher> for MatchRule {
    fn from(value: EitherQueryOrHeaderMatcher) -> Self {
        match value {
            EitherQueryOrHeaderMatcher::Header(match_rule) => match_rule,
            EitherQueryOrHeaderMatcher::Query(match_rule) => match_rule,
        }
    }
}

fn create_match_rulesets(
    route_info: &RouteInputInfo<'_>,
) -> Vec<(Option<HeadersMatchersList>, Option<QueryMatchersList>)> {
    let mut headers_cart = vec![];
    if let Some(header_matcher) = &route_info.header_matchers {
        headers_cart = header_matcher
            .0
            .catesian_product()
            .into_iter()
            .map(|rules| {
                rules
                    .into_iter()
                    .map(|rule| EitherQueryOrHeaderMatcher::Header(rule))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
    }
    let mut query_cart = vec![];
    if let Some(query_matcher) = &route_info.query_matchers {
        query_cart = query_matcher
            .0
            .catesian_product()
            .into_iter()
            .map(|rules| {
                rules
                    .into_iter()
                    .map(|rule| EitherQueryOrHeaderMatcher::Query(rule))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
    }

    if headers_cart.is_empty() && query_cart.is_empty() {
        return vec![(None, None)];
    }
    if headers_cart.is_empty() {
        let mut res = vec![];
        for matchers in query_cart {
            res.push((
                None,
                Some(QueryMatchersList(MatcherList(
                    matchers.into_iter().map(Into::into).collect(),
                ))),
            ));
        }
        return res;
    }
    if query_cart.is_empty() {
        let mut res = vec![];
        for matchers in headers_cart {
            res.push((
                Some(HeadersMatchersList(MatcherList(
                    matchers.into_iter().map(Into::into).collect(),
                ))),
                None,
            ));
        }
        return res;
    }

    let to_permute = vec![headers_cart, query_cart];

    let mut res = vec![];

    permutator::cartesian_product(
        to_permute
            .iter()
            .map(|a| a.as_slice())
            .collect::<Vec<_>>()
            .as_slice(),
        |product| {
            let mut headers_list = vec![];
            let mut query_list = vec![];
            for item in product.to_vec().into_iter().flatten() {
                match item {
                    EitherQueryOrHeaderMatcher::Header(match_rule) => {
                        headers_list.push(match_rule.clone())
                    }
                    EitherQueryOrHeaderMatcher::Query(match_rule) => {
                        query_list.push(match_rule.clone())
                    }
                }
            }
            let mut query_ruleset = None;
            let mut header_ruleset = None;
            if !query_list.is_empty() {
                query_ruleset = Some(QueryMatchersList(MatcherList(query_list)));
            }
            if !headers_list.is_empty() {
                header_ruleset = Some(HeadersMatchersList(MatcherList(headers_list)));
            }
            res.push((header_ruleset, query_ruleset));
        },
    );
    if res.is_empty() {
        return vec![(None, None)];
    }
    res
}

async fn create_http_routes(
    ctx: Arc<ctx::Context>,
    route_info: RouteInputInfo<'_>,
    http: &k8s_openapi::api::networking::v1::HTTPIngressRuleValue,
) -> anyhow::Result<Vec<HTTPRoute>> {
    let safe_hostname = utils::sanitize_hostname(&route_info.hostname);
    let gw_group = <gateways::Gateway as kube::Resource>::group(&());
    let gw_kind = <gateways::Gateway as kube::Resource>::kind(&());

    let split_routes = route_info
        .ingress_meta
        .annotations
        .as_ref()
        .and_then(|ann| ann.get(consts::SPLIT_ROUTES))
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    let match_ruleset = create_match_rulesets(&route_info);
    tracing::debug!("Match ruleset: \n{match_ruleset:#?}");

    let mut rules = vec![];

    for path in &http.paths {
        let Some(svc) = &path.backend.service else {
            tracing::warn!("Skipping backend without service");
            continue;
        };
        let Some(svc_port) = &svc.port else {
            tracing::warn!("Skipping backend without service port");
            continue;
        };
        let Some(svc_port_number) = get_svc_port_number(
            Api::namespaced(ctx.client.clone(), &route_info.ingress_namespace),
            &svc.name,
            svc_port,
        )
        .await
        else {
            tracing::warn!(
                "Skipping backend with unresolvable service port for service {}",
                &svc.name
            );
            continue;
        };
        let match_type = match path.path_type.as_str() {
            "Prefix" => HTTPRouteRulesMatchesPathType::PathPrefix,
            "Exact" => HTTPRouteRulesMatchesPathType::Exact,
            "ImplementationSpecific" => HTTPRouteRulesMatchesPathType::PathPrefix,
            _ => {
                return Err(
                    anyhow::anyhow!("Unknown path type: {}", path.path_type.as_str()).into(),
                );
            }
        };
        for (header_matchers, query_matchers) in &match_ruleset {
            rules.push(HTTPRouteRules {
                name: None,
                backend_refs: Some(
                    [HTTPRouteRulesBackendRefs {
                        name: svc.name.clone(),
                        port: Some(svc_port_number),
                        kind: None,
                        group: None,
                        namespace: None,
                        filters: None,
                        weight: None,
                    }]
                    .to_vec(),
                ),
                matches: Some(vec![HTTPRouteRulesMatches {
                    headers: header_matchers.clone().map(Into::into),
                    method: None,
                    query_params: query_matchers.clone().map(Into::into),
                    path: Some(HTTPRouteRulesMatchesPath {
                        r#type: Some(match_type.clone()),
                        value: path.path.clone(),
                    }),
                }]),
                filters: None,
                timeouts: None,
            });
        }
    }
    if rules.is_empty() {
        return Err(anyhow::anyhow!("No valid paths found").into());
    }

    // If split_routes is enabled, create a separate HTTPRoute for each rule.
    if split_routes {
        return Ok(rules
            .into_iter()
            .map(|rule| {
                HTTPRoute::new(
                    &format!(
                        "{}-{}-{}",
                        route_info.ingress_name,
                        safe_hostname,
                        utils::sanitize_hostname(
                            &rule
                                .matches
                                .as_ref()
                                .and_then(|m| m.first())
                                .and_then(|mm| mm.path.as_ref())
                                .and_then(|p| p.value.clone())
                                .unwrap_or_else(|| "root".to_string())
                        )
                    ),
                    HTTPRouteSpec {
                        hostnames: Some(vec![route_info.hostname.clone()]),
                        parent_refs: Some(
                            [HTTPRouteParentRefs {
                                group: Some(gw_group.to_string()),
                                kind: Some(gw_kind.to_string()),
                                name: route_info.gw_name.to_string(),
                                namespace: Some(route_info.gw_namespace.to_string()),
                                port: None,
                                section_name: route_info.section_name.clone(),
                            }]
                            .to_vec(),
                        ),
                        rules: Some(vec![rule]),
                    },
                )
            })
            .collect());
    }

    // Split routes is disabled, create a single HTTPRoute with all rules.
    Ok([HTTPRoute::new(
        &format!("{}-{}-http", route_info.ingress_name, safe_hostname),
        HTTPRouteSpec {
            hostnames: Some(vec![route_info.hostname.to_string()]),
            // parent_refs: None,
            parent_refs: Some(
                [HTTPRouteParentRefs {
                    group: Some(gw_group.to_string()),
                    kind: Some(gw_kind.to_string()),
                    name: route_info.gw_name.to_string(),
                    namespace: Some(route_info.gw_namespace.to_string()),
                    port: None,
                    section_name: route_info.section_name.clone(),
                }]
                .to_vec(),
            ),
            rules: Some(rules),
        },
    )]
    .to_vec())
}

async fn create_tcp_routes(
    ctx: Arc<ctx::Context>,
    route_info: RouteInputInfo<'_>,
    svc: &IngressServiceBackend,
) -> anyhow::Result<TCPRoute> {
    let safe_hostname = utils::sanitize_hostname(&route_info.hostname);
    let gw_group = <gateways::Gateway as kube::Resource>::group(&());
    let gw_kind = <gateways::Gateway as kube::Resource>::kind(&());

    let Some(svc_port) = &svc.port else {
        tracing::warn!("Skipping backend without service port");
        return Err(anyhow::anyhow!("Backend doesn't have port").into());
    };

    let Some(svc_port_number) = get_svc_port_number(
        Api::namespaced(ctx.client.clone(), &route_info.ingress_namespace),
        &svc.name,
        svc_port,
    )
    .await
    else {
        tracing::warn!(
            "skipping backend with unresolvable service port for service {}",
            &svc.name
        );
        return Err(
            anyhow::anyhow!(format!("Couldn't resolve port for a service {}", &svc.name)).into(),
        );
    };
    Ok(TCPRoute::new(
        &format!("{}-{}-tcp", route_info.ingress_name, safe_hostname),
        TCPRouteSpec {
            use_default_gateways: None,
            rules: [TCPRouteRules {
                name: None,
                backend_refs: [TCPRouteRulesBackendRefs {
                    name: svc.name.clone(),
                    port: Some(svc_port_number),
                    kind: None,
                    group: None,
                    namespace: None,
                    weight: None,
                }]
                .to_vec(),
            }]
            .to_vec(),
            parent_refs: Some(
                [TCPRouteParentRefs {
                    group: Some(gw_group.to_string()),
                    kind: Some(gw_kind.to_string()),
                    name: route_info.gw_name.to_string(),
                    namespace: Some(route_info.gw_namespace.to_string()),
                    port: None,
                    section_name: route_info.section_name.clone(),
                }]
                .to_vec(),
            ),
        },
    ))
}

#[tracing::instrument(skip(ingress, ctx), fields(ingress = ingress.name_any(), namespace = ingress.namespace()), err)]
pub async fn reconcile(ingress: Arc<Ingress>, ctx: Arc<ctx::Context>) -> I2GResult<Action> {
    if !ctx.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
        tracing::debug!("Not a leader, skipping reconciliation");
        return Ok(Action::requeue(Duration::from_secs(20)));
    }

    // Only translate if the annotation is present and true
    // or if skip_by_default is false and
    // the annotation is not present or equals to true
    let skip_translation = ingress
        .meta()
        .annotations
        .as_ref()
        .and_then(|ann| ann.get(consts::TRANSLATE_INGRESS))
        .map(|v| v.to_lowercase() != "true")
        .unwrap_or(ctx.args.skip_by_default);

    if skip_translation {
        tracing::info!("Skipping translation due to annotation or operator settings");
        return Ok(Action::requeue(Duration::from_secs(60)));
    }

    tracing::info!("Reconciling Ingress");
    let ingress_spec = ingress
        .spec
        .as_ref()
        .ok_or(anyhow::anyhow!("Ingres doesn't have spec section"))?;
    let ingress_rules = ingress_spec
        .rules
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Ingress doesn't have any routing rules"))?;
    let ingress_namespace = ingress
        .namespace()
        .ok_or_else(|| anyhow::anyhow!("Ingress doesn't have a namespace"))?;

    let desired_section_name = ingress
        .meta()
        .annotations
        .as_ref()
        .and_then(|ann| ann.get(consts::DESIRED_SECTION))
        .cloned();

    let gw_namespace = ingress
        .meta()
        .annotations
        .as_ref()
        .and_then(|annot| annot.get(consts::GATEWAY_NAMESPACE))
        .unwrap_or(&ctx.args.default_gateway_namespace);

    let gw_name = ingress
        .meta()
        .annotations
        .as_ref()
        .and_then(|annot| annot.get(consts::GATEWAY_NAME))
        .unwrap_or(&ctx.args.default_gateway_name);

    let header_matchers = ingress
        .meta()
        .annotations
        .as_ref()
        .map(|annotations| {
            MatcherList::from_annotations(annotations, consts::HEADER_FILTERS_PREFIX)
        })
        .map(HeadersMatchersList);
    let query_matchers = ingress
        .meta()
        .annotations
        .as_ref()
        .map(|annotations| MatcherList::from_annotations(annotations, consts::QUERY_FILTERS_PREFIX))
        .map(QueryMatchersList);

    let default_backend = ingress_spec.default_backend.as_ref();

    for rule in ingress_rules {
        let Some(host) = &rule.host else {
            tracing::warn!("Skipping rule without host");
            continue;
        };

        let route_info = RouteInputInfo {
            ingress_name: ingress.name_any(),
            header_matchers: header_matchers.clone(),
            query_matchers: query_matchers.clone(),
            gw_name: gw_name.to_string(),
            gw_namespace: gw_namespace.to_string(),
            ingress_meta: ingress.meta(),
            hostname: host.to_string(),
            ingress_namespace: ingress_namespace.clone(),
            section_name: desired_section_name.clone(),
        };

        if let Some(http) = &rule.http {
            let Ok(routes) = create_http_routes(ctx.clone(), route_info, &http).await else {
                tracing::warn!("Failed to create HTTPRoute for host {}", host);
                continue;
            };
            for mut route in routes {
                if ctx.args.link_to_ingress {
                    route.meta_mut().add_owner(ingress.as_ref());
                }
                Api::<HTTPRoute>::namespaced(ctx.client.clone(), &ingress_namespace)
                    .patch(
                        &route.name_any(),
                        &PatchParams {
                            field_manager: Some("ingress-to-gateway-controller".to_string()),
                            ..PatchParams::default()
                        },
                        &kube::api::Patch::Apply(route),
                    )
                    .instrument(tracing::info_span!("Applying generated HTTPRoute"))
                    .await?;
            }
        } else {
            if !ctx.args.experimental {
                tracing::warn!(
                    "Skipping rule non-http rule. In order to migrate it to TCPRoute, please add --experimental flag to i2g-operator."
                );
                continue;
            }
            // In case if rule.http is None
            let Some(backend) = default_backend else {
                tracing::warn!("Skipping non-HTTP Ingress rule without default backend");
                continue;
            };
            let Some(backend_svc) = &backend.service else {
                tracing::warn!("defaultBackend doesn't have a service, skipping.");
                continue;
            };

            let Ok(mut route) = create_tcp_routes(ctx.clone(), route_info, backend_svc).await
            else {
                tracing::warn!("Failed to create TCPRoute for host {}", host);
                continue;
            };

            if ctx.args.link_to_ingress {
                route.meta_mut().add_owner(ingress.as_ref());
            }

            Api::<TCPRoute>::namespaced(ctx.client.clone(), &ingress_namespace)
                .patch(
                    &route.name_any(),
                    &PatchParams {
                        field_manager: Some("ingress-to-gateway-controller".to_string()),
                        ..PatchParams::default()
                    },
                    &kube::api::Patch::Apply(route),
                )
                .instrument(tracing::info_span!("Applying generated TCPRoute"))
                .await?;
        }
    }

    Ok(Action::requeue(Duration::from_secs(10)))
}

#[tracing::instrument(skip(obj, _ctx), fields(ingress = obj.name_any()))]
fn on_error(obj: Arc<Ingress>, _err: &I2GError, _ctx: Arc<ctx::Context>) -> Action {
    Action::requeue(Duration::from_secs(30))
}

async fn lease_renew(ctx: Arc<ctx::Context>) {
    let leadership = kube_leader_election::LeaseLock::new(
        ctx.client.clone(),
        ctx.client.default_namespace(),
        kube_leader_election::LeaseLockParams {
            holder_id: ctx.hostname.clone(),
            lease_name: "i2g-operator-lock".into(),
            lease_ttl: Duration::from_secs(15),
        },
    );
    loop {
        match leadership.try_acquire_or_renew().await {
            Ok(lease) => {
                if lease.acquired_lease {
                    tracing::info!("Acquired leadership lease");
                }
                ctx.is_leader
                    .store(lease.acquired_lease, std::sync::atomic::Ordering::Relaxed)
            }
            Err(err) => {
                tracing::warn!("Failed to acquire or renew lease: {}", err);
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let ctx = Arc::new(ctx::Context::new().await?);
    tracing_subscriber::fmt()
        .with_max_level(ctx.args.log_level)
        .init();
    tracing::info!("Staring operator");
    tracing::info!("CLI argument: {:?}", ctx.args);

    let lease_renewer = lease_renew(ctx.clone());

    let ingress_controller = kube::runtime::Controller::new(
        Api::<Ingress>::all(ctx.client.clone()),
        kube::runtime::watcher::Config::default(),
    )
    .run(reconcile, on_error, ctx.clone())
    .for_each(|_| futures::future::ready(()));

    tokio::select! {
        _ = lease_renewer => {
            tracing::error!("Lease renewer task exited unexpectedly");
        },
        _ = ingress_controller => {
            tracing::error!("Ingress controller task exited unexpectedly");
        },
    }

    Ok(())
}

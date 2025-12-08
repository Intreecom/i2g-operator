use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use gateway_api::{
    gateways,
    httproutes::{
        HTTPRoute, HTTPRouteParentRefs, HTTPRouteRules, HTTPRouteRulesBackendRefs,
        HTTPRouteRulesMatches, HTTPRouteRulesMatchesPath, HTTPRouteRulesMatchesPathType,
        HTTPRouteSpec,
    },
};
use k8s_openapi::api::{
    core::v1::Service,
    networking::v1::{Ingress, ServiceBackendPort},
};
use kube::{Api, Resource, ResourceExt, api::PatchParams, runtime::controller::Action};

use crate::{
    err::{I2GError, I2GResult},
    utils::ObjectMetaI2GExt,
};

mod args;
mod ctx;
mod err;
mod utils;

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

async fn create_http_route(
    ctx: Arc<ctx::Context>,
    ingress_namespace: &str,
    http: &k8s_openapi::api::networking::v1::HTTPIngressRuleValue,
    hostname: &str,
) -> anyhow::Result<HTTPRoute> {
    let safe_hostname = hostname.replace('.', "-");
    let gw_group = <gateways::Gateway as kube::Resource>::group(&());
    let gw_kind = <gateways::Gateway as kube::Resource>::kind(&());

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
            Api::namespaced(ctx.client.clone(), ingress_namespace),
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
        let mut path_matches = vec![];
        for path in &http.paths {
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
            path_matches.push(HTTPRouteRulesMatches {
                headers: None,
                method: None,
                query_params: None,
                path: Some(HTTPRouteRulesMatchesPath {
                    r#type: Some(match_type),
                    value: path.path.clone(),
                }),
            });
        }
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
            matches: Some(path_matches),
            filters: None,
            timeouts: None,
        });
    }
    if rules.is_empty() {
        return Err(anyhow::anyhow!("No valid paths found").into());
    }
    let route_name = format!("{safe_hostname}-http");
    Ok(HTTPRoute::new(
        &route_name,
        HTTPRouteSpec {
            hostnames: Some(vec![hostname.to_string()]),
            // parent_refs: None,
            parent_refs: Some(
                [HTTPRouteParentRefs {
                    group: Some(gw_group.to_string()),
                    kind: Some(gw_kind.to_string()),
                    name: ctx.args.default_gateway_name.clone(),
                    namespace: Some(ctx.args.default_gateway_namespace.clone()),
                    port: Some(80),
                    section_name: None,
                }]
                .to_vec(),
            ),
            rules: Some(rules),
        },
    ))
}

#[tracing::instrument(skip(ingress, ctx), fields(ingress = ingress.name_any(), namespace = ingress.namespace()), err)]
pub async fn reconcile(ingress: Arc<Ingress>, ctx: Arc<ctx::Context>) -> I2GResult<Action> {
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

    for rule in ingress_rules {
        let Some(host) = &rule.host else {
            tracing::warn!("Skipping rule without host");
            continue;
        };
        if let Some(http) = &rule.http {
            let Ok(mut route) =
                create_http_route(ctx.clone(), &ingress_namespace, &http, &host).await
            else {
                tracing::warn!("Failed to create HTTPRoute for host {}", host);
                continue;
            };
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
                .await?;
        } else {
            // In case if rule.http is None
            unimplemented!("Only HTTP Ingress rules are supported for now");
        }
    }

    Ok(Action::requeue(Duration::from_secs(10)))
}

#[tracing::instrument(skip(obj, _ctx), fields(ingress = obj.name_any()))]
fn on_error(obj: Arc<Ingress>, _err: &I2GError, _ctx: Arc<ctx::Context>) -> Action {
    Action::requeue(Duration::from_secs(30))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let ctx = Arc::new(ctx::Context::new().await?);

    kube::runtime::Controller::new(
        Api::<Ingress>::all(ctx.client.clone()),
        kube::runtime::watcher::Config::default(),
    )
    .run(reconcile, on_error, ctx.clone())
    .for_each(|_| futures::future::ready(()))
    .await;

    Ok(())
}

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
use kube::{Api, Resource, ResourceExt, api::PatchParams, runtime::controller::Action};
use tracing::Instrument;

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
    ingress_name: &str,
    section_name: Option<&String>,
    ingress_namespace: &str,
    http: &k8s_openapi::api::networking::v1::HTTPIngressRuleValue,
    hostname: &str,
) -> anyhow::Result<HTTPRoute> {
    let safe_hostname = utils::sanitize_hostname(hostname);
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
    let route_name = format!("{ingress_name}-{safe_hostname}-http");
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
                    port: None,
                    section_name: section_name.cloned(),
                }]
                .to_vec(),
            ),
            rules: Some(rules),
        },
    ))
}

async fn create_tcp_route(
    ctx: Arc<ctx::Context>,
    ingress_name: &str,
    section_name: Option<&String>,
    namespace: &str,
    svc: &IngressServiceBackend,
    hostname: &str,
) -> anyhow::Result<TCPRoute> {
    let safe_hostname = utils::sanitize_hostname(hostname);
    let gw_group = <gateways::Gateway as kube::Resource>::group(&());
    let gw_kind = <gateways::Gateway as kube::Resource>::kind(&());

    let Some(svc_port) = &svc.port else {
        tracing::warn!("Skipping backend without service port");
        return Err(anyhow::anyhow!("Backend doesn't have port").into());
    };

    let Some(svc_port_number) = get_svc_port_number(
        Api::namespaced(ctx.client.clone(), namespace),
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
        &format!("{ingress_name}-{safe_hostname}-tcp"),
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
                    name: ctx.args.default_gateway_name.clone(),
                    namespace: Some(ctx.args.default_gateway_namespace.clone()),
                    port: None,
                    section_name: section_name.cloned(),
                }]
                .to_vec(),
            ),
        },
    ))
}

#[tracing::instrument(skip(ingress, ctx), fields(ingress = ingress.name_any(), namespace = ingress.namespace()), err)]
pub async fn reconcile(ingress: Arc<Ingress>, ctx: Arc<ctx::Context>) -> I2GResult<Action> {
    if ctx.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
        tracing::debug!("Not a leader, skipping reconciliation");
        return Ok(Action::requeue(Duration::from_secs(20)));
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
        .and_then(|ann| ann.get("i2g-operator/sectionName"))
        .cloned();

    let default_backend = ingress_spec.default_backend.as_ref();

    for rule in ingress_rules {
        let Some(host) = &rule.host else {
            tracing::warn!("Skipping rule without host");
            continue;
        };

        if let Some(http) = &rule.http {
            let Ok(mut route) = create_http_route(
                ctx.clone(),
                &ingress.name_any(),
                desired_section_name.as_ref(),
                &ingress_namespace,
                &http,
                &host,
            )
            .await
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
                .instrument(tracing::info_span!("Applying generated HTTPRoute"))
                .await?;
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

            let Ok(mut route) = create_tcp_route(
                ctx.clone(),
                &ingress.name_any(),
                desired_section_name.as_ref(),
                &ingress_namespace,
                backend_svc,
                &host,
            )
            .await
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

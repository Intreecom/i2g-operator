# ingress2gateway operator

> [!IMPORTANT]
> This project is not intended to use for your applications since you have direct access to this config. It's main goal is to convert third-party ingresses to gateways.

### Installation

We recommend using our helm chart in order to install this project.

```bash
helm upgrade --install --namespace i2g --create-namespace i2g  oci://ghcr.io/intreecom/charts/i2g-operator -f values.yaml
```

You can see our values file for helm chart by running this command:

```bash
helm show values oci://ghcr.io/intreecom/charts/i2g-operator
```

Sources of the helm chart are located here in our chart museum: https://github.com/Intreecom/charts/

## Configuration

It can be configured using CLI argument or by env variables.

```bash
# Log level of the operator
I2G_LOG_LEVEL="info"
# Wether to enable experimental channel for gateway-api
# If it's true, then ingresses that don't have
# `http` in their rules will be translated to TCPRoute
# instead of HTTPRoute.
I2G_EXPERIMENTAL="true"
# Whether to link created resources to the ingress
# which was used as a source for generation.

# It's useful if you want to delete all HTTPRoute and
# TCPRoutes when ingress is being deleted.
I2G_LINK_TO_INGRESS="true"
# Name of the gateway routes will be linked to.
I2G_DEFAULT_GATEWAY_NAME="gw"
# Namespace where gateway is located.
I2G_DEFAULT_GATEWAY_NAMESPACE="default"
# If true, then I2G will be skipping ingresses,
# unless they have `i2g-operator/translate: "true"` annotation.
I2G_SKIP_BY_DEFAULT="false"
```

Also amost all those configuration variables can be overwritten by ingress annotations
and there are few annotations with special behaviour.


```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  annotations:
    # Will create HTTPRoute for each path of each rule.
    # This is required for some ingresses, because they
    # contain to many path rules.
    # HTTPRoute rules can only have 16 match rules at most.
    i2g-operator/split-paths: "true"
    # If false, will not translate this ingress resource.
    i2g-operator/translate: "true"
    # Override default gateway's name for generated resources.
    i2g-operator/gateway-name: "other-gw"
    # Override default gateway's namespace for generated resources.
    i2g-operator/gateway-namespace: "my-ns"
    # Specify a particular listener name
    # for generated routes.
    i2g-operator/section-name: "my-section"
  name: test-ingress
spec:
  ingressClassName: nginx
  rules:
  - host: test.localhost
    http:
      paths:
      - backend:
          service:
            name: my-svc
            port:
              number: 3100
        path: /api/memes
        pathType: Prefix
  tls:
  - hosts:
    - test.localhost
    secretName: test-tls-secret

```

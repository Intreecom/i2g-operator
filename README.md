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

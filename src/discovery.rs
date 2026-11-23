use crate::{coordinator::Endpoint, transport::Error};
#[derive(serde::Deserialize)]
pub struct Kubernetes {
    pub api_url: String,
    pub ca_file: String,
    pub token_file: String,
    pub namespace: String,
    pub service_name: String,
    pub port_name: String,
}
#[derive(serde::Deserialize)]
struct List {
    items: Vec<Slice>,
}
#[derive(serde::Deserialize)]
struct Slice {
    endpoints: Vec<Entry>,
    ports: Vec<Port>,
}
#[derive(serde::Deserialize)]
struct Port {
    name: Option<String>,
    port: Option<u16>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    hostname: Option<String>,
    addresses: Vec<String>,
    target_ref: Option<Target>,
    conditions: Conditions,
}
#[derive(serde::Deserialize)]
struct Target {
    kind: Option<String>,
    uid: Option<String>,
}
#[derive(serde::Deserialize)]
struct Conditions {
    ready: Option<bool>,
    terminating: Option<bool>,
}
impl Kubernetes {
    pub async fn endpoints(&self) -> Result<Vec<Endpoint>, Error> {
        for name in [&self.namespace, &self.service_name] {
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return Err("invalid Kubernetes DNS name".into());
            }
        }
        let ca = reqwest::Certificate::from_pem(&tokio::fs::read(&self.ca_file).await?)?;
        let token = tokio::fs::read_to_string(&self.token_file).await?;
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .https_only(true)
            .tls_built_in_root_certs(false)
            .add_root_certificate(ca)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let mut url = reqwest::Url::parse(&format!(
            "{}/apis/discovery.k8s.io/v1/namespaces/{}/endpointslices",
            self.api_url.trim_end_matches('/'),
            self.namespace
        ))?;
        url.query_pairs_mut().append_pair(
            "labelSelector",
            &format!("kubernetes.io/service-name={}", self.service_name),
        );
        let response = client
            .get(url)
            .bearer_auth(token.trim())
            .send()
            .await?
            .error_for_status()?;
        let list: List = serde_json::from_slice(&response.bytes().await?)?;
        let mut endpoints = std::collections::BTreeMap::new();
        for slice in list.items {
            let port = slice
                .ports
                .iter()
                .find(|p| p.name.as_deref() == Some(&self.port_name))
                .and_then(|p| p.port);
            let Some(port) = port else {
                continue;
            };
            for entry in slice.endpoints {
                let Some(target) = entry.target_ref else {
                    continue;
                };
                if target.kind.as_deref() != Some("Pod") {
                    continue;
                }
                let Some(uid) = target.uid else {
                    continue;
                };
                if uid.is_empty() {
                    continue;
                }
                let host = if let Some(hostname) = entry.hostname.filter(|name| {
                    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }) {
                    format!("{}.{}.{}.svc", hostname, self.service_name, self.namespace)
                } else {
                    let address = entry
                        .addresses
                        .first()
                        .ok_or("EndpointSlice address missing")?
                        .parse::<std::net::IpAddr>()?;
                    match address {
                        std::net::IpAddr::V4(ip) => ip.to_string(),
                        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
                    }
                };
                // The advertised DNS hostname or IP must be covered by the server certificate.
                let url = format!("https://{host}:{port}");
                endpoints.insert(
                    uid.clone(),
                    Endpoint {
                        instance_id: uid,
                        url,
                        ready: entry.conditions.ready == Some(true)
                            && entry.conditions.terminating != Some(true),
                    },
                );
            }
        }
        Ok(endpoints.into_values().collect())
    }
}

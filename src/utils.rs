use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::{ResourceExt, api::ObjectMeta};

pub trait ObjectMetaI2GExt: Default {
    fn add_owner<T>(&mut self, owner: &T)
    where
        T: kube::Resource<DynamicType = ()>,
        T::DynamicType: Eq + std::hash::Hash + Clone;
}

impl ObjectMetaI2GExt for ObjectMeta {
    fn add_owner<T>(&mut self, owner: &T)
    where
        T: kube::Resource<DynamicType = ()>,
        T::DynamicType: Eq + std::hash::Hash + Clone,
    {
        let mut owners = self.owner_references.take().unwrap_or_default();

        let owner = OwnerReference {
            api_version: String::from(T::api_version(&())),
            kind: String::from(T::kind(&())),
            name: owner.name_any(),
            uid: String::from(owner.meta().uid.as_ref().unwrap()),
            controller: None,
            block_owner_deletion: Some(false),
        };
        if owners.iter().any(|o| o.uid == owner.uid) {
            // already present
            self.owner_references = Some(owners);
            return;
        }
        owners.push(owner);
        self.owner_references = Some(owners);
    }
}

pub fn sanitize_hostname(hostname: &str) -> String {
    let re = regex::Regex::new("[^a-zA-Z0-9]+").unwrap();
    let sanitized_str = re.replace_all(hostname, "-");
    let res = sanitized_str
        .trim()
        .trim_start_matches("-")
        .trim_end_matches("-");
    if res.is_empty() || res == "*" {
        return "all-hosts".to_string();
    }
    res.to_string()
}

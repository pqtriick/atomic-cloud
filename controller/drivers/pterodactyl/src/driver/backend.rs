use std::path::Path;

use allocation::BAllocation;
use anyhow::Result;
use colored::Colorize;
use common::{BBody, BList, BObject};
use node::BNode;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use server::{BCServer, BCServerAllocation, BServer, BServerFeatureLimits};
use user::BUser;

use crate::{
    config::{LoadFromTomlFile, SaveToTomlFile, CONFIG_DIRECTORY},
    debug, error,
    exports::node::driver::bridge::Server,
    node::driver::http::{send_http_request, Header, Method, Response},
    warn,
};

pub mod allocation;
mod common;
mod node;
pub mod server;
mod user;

const BACKEND_FILE: &str = "backend.toml";

/* Endpoints */
const APPLICATION_ENDPOINT: &str = "/api/application";

#[derive(Deserialize, Serialize)]
pub struct ResolvedValues {
    pub user: u32,
}

#[derive(Deserialize, Serialize)]
pub struct Backend {
    url: Option<String>,
    token: Option<String>,
    user: Option<String>,
    resolved: Option<ResolvedValues>,
}

impl ResolvedValues {
    fn new_resolved(backend: &Backend) -> Result<Self> {
        let user = backend
            .get_user_by_name(backend.user.as_ref().unwrap())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "The provided user {} does not exist in the Pterodactyl panel",
                    &backend.user.as_ref().unwrap()
                )
            })?
            .id;
        Ok(Self { user })
    }
}

impl Backend {
    fn new_empty() -> Self {
        Self {
            url: Some("".to_string()),
            token: Some("".to_string()),
            user: Some("".to_string()),
            resolved: None,
        }
    }

    fn load_or_empty() -> Self {
        let path = Path::new(CONFIG_DIRECTORY).join(BACKEND_FILE);
        if path.exists() {
            Self::load_from_file(&path).unwrap_or_else(|err| {
                warn!("Failed to read backend configuration from file: {}", err);
                Self::new_empty()
            })
        } else {
            let backend = Self::new_empty();
            if let Err(error) = backend.save_to_file(&path, false) {
                error!(
                    "Failed to save default backend configuration to file: {}",
                    &error
                );
            }
            backend
        }
    }

    fn new_filled() -> Result<Self> {
        let mut backend = Self::load_or_empty();

        // Check config values are overridden by environment variables
        if let Ok(url) = std::env::var("PTERODACTYL_URL") {
            backend.url = Some(url);
        }
        if let Ok(token) = std::env::var("PTERODACTYL_TOKEN") {
            backend.token = Some(token);
        }
        if let Ok(user) = std::env::var("PTERODACTYL_USER") {
            backend.user = Some(user);
        }

        let mut missing = vec![];
        if backend.url.is_none() || backend.url.as_ref().unwrap().is_empty() {
            missing.push("url");
        }
        if backend.token.is_none() || backend.token.as_ref().unwrap().is_empty() {
            missing.push("token");
        }
        if backend.user.is_none() || backend.user.as_ref().unwrap().is_empty() {
            missing.push("user");
        }
        if !missing.is_empty() {
            error!(
                "The following required configuration values are missing: {}",
                missing.join(", ").red()
            );
            return Err(anyhow::anyhow!("Missing required configuration values"));
        }

        Ok(backend)
    }

    pub fn new_filled_and_resolved() -> Result<Self> {
        let mut backend = Self::new_filled()?;
        match ResolvedValues::new_resolved(&backend) {
            Ok(resolved) => backend.resolved = Some(resolved),
            Err(error) => return Err(error),
        }
        Ok(backend)
    }

    pub fn create_server(
        &self,
        server: &Server,
        allocation: &BAllocation,
        egg: u32,
        startup: &str,
        features: BServerFeatureLimits,
    ) -> Option<BServer> {
        let backend_server = BCServer {
            name: server.name.clone(),
            user: self.resolved.as_ref().unwrap().user,
            egg,
            docker_image: server.allocation.deployment.image.clone(),
            startup: startup.to_owned(),
            environment: server
                .allocation
                .deployment
                .environment
                .iter()
                .map(|value| (value.key.clone(), value.value.clone()))
                .collect(),
            limits: server.allocation.resources.into(),
            feature_limits: features,
            allocation: BCServerAllocation {
                default: allocation.id,
            },
        };
        self.post_object_to_api::<BCServer, BServer>(
            APPLICATION_ENDPOINT,
            "servers",
            &BObject {
                attributes: backend_server,
            },
        )
        .map(|data| data.attributes)
    }

    pub fn get_server_by_name(&self, name: &str) -> Option<BServer> {
        self.api_find_on_pages::<BServer>(Method::Get, APPLICATION_ENDPOINT, "servers", |object| {
            object
                .data
                .iter()
                .find(|server| server.attributes.name == name)
                .map(|server| server.attributes.clone())
        })
    }

    pub fn get_free_allocations(
        &self,
        used_allocations: &[BAllocation],
        node_id: u32,
        amount: u32,
    ) -> Vec<BAllocation> {
        let mut allocations = Vec::with_capacity(amount as usize);
        self.for_each_on_pages::<BAllocation>(
            Method::Get,
            APPLICATION_ENDPOINT,
            format!("nodes/{}/allocations", &node_id).as_str(),
            |response| {
                for allocation in &response.data {
                    if allocation.attributes.assigned
                        || used_allocations.iter().any(|used| {
                            used.ip == allocation.attributes.ip
                                && used.port == allocation.attributes.port
                        })
                    {
                        continue;
                    }
                    allocations.push(allocation.attributes.clone());
                    if allocations.len() >= amount as usize {
                        return true;
                    }
                }
                false
            },
        );
        allocations
    }

    pub fn get_user_by_name(&self, username: &str) -> Option<BUser> {
        self.api_find_on_pages::<BUser>(Method::Get, APPLICATION_ENDPOINT, "users", |object| {
            object
                .data
                .iter()
                .find(|node| node.attributes.username == username)
                .map(|node| node.attributes.clone())
        })
    }

    pub fn get_node_by_name(&self, name: &str) -> Option<BNode> {
        self.api_find_on_pages::<BNode>(Method::Get, APPLICATION_ENDPOINT, "nodes", |object| {
            object
                .data
                .iter()
                .find(|node| node.attributes.name == name)
                .map(|node| node.attributes.clone())
        })
    }

    fn api_find_on_pages<T: DeserializeOwned>(
        &self,
        method: Method,
        endpoint: &str,
        target: &str,
        mut callback: impl FnMut(&BBody<Vec<BObject<T>>>) -> Option<T>,
    ) -> Option<T> {
        let mut value = None;
        self.for_each_on_pages(method, endpoint, target, |response| {
            if let Some(data) = callback(response) {
                value = Some(data);
                return true;
            }
            false
        });
        value
    }

    fn for_each_on_pages<T: DeserializeOwned>(
        &self,
        method: Method,
        endpoint: &str,
        target: &str,
        mut callback: impl FnMut(&BBody<Vec<BObject<T>>>) -> bool,
    ) {
        let mut page = 1;
        loop {
            if let Some(response) = self.api_get_list::<T>(method, endpoint, target, Some(page)) {
                if callback(&response) {
                    return;
                }
                if response.meta.pagination.total_pages <= page {
                    break;
                }
                page += 1;
            }
        }
    }

    fn api_get_list<T: DeserializeOwned>(
        &self,
        method: Method,
        endpoint: &str,
        target: &str,
        page: Option<u32>,
    ) -> Option<BList<T>> {
        self.send_request_to_api(method, endpoint, target, None, page)
    }

    fn post_object_to_api<T: Serialize, K: DeserializeOwned>(
        &self,
        endpoint: &str,
        target: &str,
        object: &BObject<T>,
    ) -> Option<BObject<K>> {
        let body = serde_json::to_vec(object).ok();
        self.send_request_to_api(Method::Post, endpoint, target, body.as_deref(), None)
    }

    fn send_request_to_api<T: DeserializeOwned>(
        &self,
        method: Method,
        endpoint: &str,
        target: &str,
        body: Option<&[u8]>,
        page: Option<u32>,
    ) -> Option<T> {
        let mut url = format!("{}{}/{}", &self.url.as_ref().unwrap(), endpoint, target);
        if let Some(page) = page {
            url = format!("{}?page={}", &url, &page);
        }
        debug!(
            "Sending request to the pterodactyl panel: {:?} {}",
            method, &url
        );
        let response = send_http_request(
            method,
            &url,
            &[Header {
                key: "Authorization".to_string(),
                value: format!("Bearer {}", &self.token.as_ref().unwrap()),
            }],
            body,
        );
        if let Some(response) = Self::handle_response::<T>(response, 200) {
            return Some(response);
        }
        None
    }

    fn handle_response<T: DeserializeOwned>(
        response: Option<Response>,
        expected_code: u32,
    ) -> Option<T> {
        response.as_ref()?;
        let response = response.unwrap();
        if response.status_code != expected_code {
            error!(
                "Received {} status code {} from the pterodactyl panel: {}",
                "unexpected".red(),
                &response.status_code,
                &response.reason_phrase
            );
            debug!(
                "Response body: {}",
                String::from_utf8_lossy(&response.bytes)
            );
            return None;
        }
        let response = serde_json::from_slice::<T>(&response.bytes);
        if let Err(error) = response {
            error!(
                "{} to parse response from the pterodactyl panel: {}",
                "Failed".red(),
                &error
            );
            return None;
        }
        Some(response.unwrap())
    }
}

impl SaveToTomlFile for Backend {}
impl LoadFromTomlFile for Backend {}

//! Asynchronous Zabbix API client.
//!
//! Enable this module with the crate's `async` feature. Unlike the blocking
//! client, an authenticated instance created with username/password must be
//! logged out explicitly with [`ZabbixInstance::logout`], because Rust does
//! not support asynchronous work in [`Drop`].

use reqwest::Client;
use semver::{Version, VersionReq};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    ApiRequestParams, AuthType, ZabbixError, parse_params, parse_response, prepare_request,
};

/// Represents an asynchronous connection to a Zabbix server.
pub struct ZabbixInstance {
    id: String,
    url: String,
    token: String,
    request_client: Client,
    need_auth_in_body: bool,
    version: String,
    need_logout: bool,
}

impl ZabbixInstance {
    /// Creates a builder for an asynchronous Zabbix client.
    pub fn builder(url: &str) -> ZabbixInstanceBuilder {
        ZabbixInstanceBuilder::new(url)
    }

    /// Returns the internally generated UUID for this instance.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the Zabbix base URL used by this instance.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Logs out and invalidates a username/password session token.
    ///
    /// Token-authenticated instances do not need to log out. Call this method
    /// explicitly before dropping a username/password-authenticated instance.
    pub async fn logout(&mut self) -> Result<&mut Self, ZabbixError> {
        if !self.need_logout {
            return Ok(self);
        }

        Self::zabbix_raw_request(
            &self.request_client,
            &self.url,
            "user.logout",
            serde_json::json!([]),
            &self.token,
            self.need_auth_in_body,
        )
        .await?;

        self.token.clear();
        self.need_logout = false;
        Ok(self)
    }

    /// Retrieves the Zabbix server API version asynchronously.
    pub async fn get_version(&self) -> Result<String, ZabbixError> {
        Self::zabbix_raw_request(
            &self.request_client,
            &self.url,
            "apiinfo.version",
            serde_json::json!([]),
            "",
            false,
        )
        .await
    }

    /// Checks the discovered Zabbix version against a semantic version requirement.
    pub fn check_version(&self, version_req: &str) -> Result<bool, ZabbixError> {
        let version_req = VersionReq::parse(version_req)?;
        let current_version = Version::parse(&self.version)?;
        Ok(version_req.matches(&current_version))
    }

    /// Makes an asynchronous JSON-RPC request to the Zabbix API.
    pub async fn zabbix_request<P: Into<ApiRequestParams>>(
        &self,
        method: &str,
        params: P,
    ) -> Result<String, ZabbixError> {
        let params = parse_params(params)?;

        Self::zabbix_raw_request(
            &self.request_client,
            &self.url,
            method,
            params,
            &self.token,
            self.need_auth_in_body,
        )
        .await
    }

    async fn zabbix_raw_request(
        client: &Client,
        url: &str,
        method: &str,
        params: Value,
        token: &str,
        need_auth_in_body: bool,
    ) -> Result<String, ZabbixError> {
        let (payload, authorization) = prepare_request(method, params, token, need_auth_in_body);

        let mut request_builder = client
            .post(format!("{url}/api_jsonrpc.php"))
            .header("Content-Type", "application/json-rpc");

        if let Some(value) = authorization {
            request_builder = request_builder.header("Authorization", value);
        }

        let response = request_builder.json(&payload).send().await?;

        if !response.status().is_success() {
            return Err(ZabbixError::Other(format!(
                "HTTP Error: {}",
                response.status()
            )));
        }

        parse_response(&response.text().await?)
    }
}

/// Builder for an asynchronous [`ZabbixInstance`].
pub struct ZabbixInstanceBuilder {
    url: String,
    accept_invalid_certs: bool,
    client: Option<Client>,
    need_auth_in_body: bool,
    version: String,
}

impl ZabbixInstanceBuilder {
    /// Creates a builder using the Zabbix base URL.
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            accept_invalid_certs: false,
            client: None,
            need_auth_in_body: false,
            version: String::new(),
        }
    }

    /// Configures whether the client accepts invalid TLS certificates.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Builds the HTTP client and discovers the Zabbix API version.
    pub async fn build(mut self) -> Result<Self, ZabbixError> {
        let client = Client::builder()
            .danger_accept_invalid_certs(self.accept_invalid_certs)
            .build()?;

        let version = ZabbixInstance::zabbix_raw_request(
            &client,
            &self.url,
            "apiinfo.version",
            serde_json::json!([]),
            "",
            false,
        )
        .await?;
        let version = version.trim_matches('"');
        let current_version = Version::parse(version)?;

        self.need_auth_in_body = !VersionReq::parse(">=6.4")?.matches(&current_version);
        self.client = Some(client);
        self.version = version.to_string();

        Ok(self)
    }

    /// Authenticates with an API token or username/password credentials.
    pub async fn login(self, auth_type: AuthType) -> Result<ZabbixInstance, ZabbixError> {
        match auth_type {
            AuthType::Token(token) => self.login_with_token(token).await,
            AuthType::UsernamePassword(username, password) => {
                self.login_with_username_password(username, password).await
            }
        }
    }

    async fn login_with_token(self, token: String) -> Result<ZabbixInstance, ZabbixError> {
        let client = self.client.ok_or_else(|| {
            ZabbixError::Other("Client not initialized. Did you call build()?".to_string())
        })?;

        if let Err(error) = ZabbixInstance::zabbix_raw_request(
            &client,
            &self.url,
            "user.checkAuthentication",
            serde_json::json!({"token": token}),
            "",
            self.need_auth_in_body,
        )
        .await
        {
            return Err(ZabbixError::ApiError {
                message: "Invalid token".to_string(),
                data: error.to_string(),
            });
        }

        Ok(ZabbixInstance {
            id: Uuid::new_v4().to_string(),
            url: self.url,
            token,
            request_client: client,
            need_auth_in_body: self.need_auth_in_body,
            version: self.version,
            need_logout: false,
        })
    }

    async fn login_with_username_password(
        self,
        username: String,
        password: String,
    ) -> Result<ZabbixInstance, ZabbixError> {
        let current_version = Version::parse(&self.version)?;
        let user_parameter = if current_version <= Version::parse("5.2.0")? {
            "user"
        } else {
            "username"
        };

        let client = self.client.ok_or_else(|| {
            ZabbixError::Other("Client not initialized. Did you call build()?".to_string())
        })?;

        let token = ZabbixInstance::zabbix_raw_request(
            &client,
            &self.url,
            "user.login",
            serde_json::json!({user_parameter: username, "password": password}),
            "",
            self.need_auth_in_body,
        )
        .await?;

        Ok(ZabbixInstance {
            id: Uuid::new_v4().to_string(),
            url: self.url,
            token,
            request_client: client,
            need_auth_in_body: self.need_auth_in_body,
            version: self.version,
            need_logout: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use mockito::{Matcher, Server};

    use super::*;

    #[tokio::test]
    async fn token_login_and_authenticated_request_succeed() {
        let mut server = Server::new_async().await;
        let url = server.url();

        let version = server
            .mock("POST", "/api_jsonrpc.php")
            .match_body(Matcher::PartialJson(
                serde_json::json!({"method": "apiinfo.version"}),
            ))
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create_async()
            .await;
        let authentication = server
            .mock("POST", "/api_jsonrpc.php")
            .match_body(Matcher::PartialJson(
                serde_json::json!({"method": "user.checkAuthentication"}),
            ))
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"valid","id":1}"#)
            .create_async()
            .await;
        let request = server
            .mock("POST", "/api_jsonrpc.php")
            .match_header("authorization", "Bearer test_token")
            .match_body(Matcher::PartialJson(
                serde_json::json!({"method": "host.get"}),
            ))
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":[{"host":"web-01"}],"id":1}"#)
            .create_async()
            .await;

        let zabbix = ZabbixInstance::builder(&url)
            .build()
            .await
            .unwrap()
            .login(AuthType::Token("test_token".to_string()))
            .await
            .unwrap();
        let result = zabbix
            .zabbix_request("host.get", serde_json::json!({"output": ["host"]}))
            .await
            .unwrap();

        assert_eq!(result, r#"[{"host":"web-01"}]"#);
        assert!(zabbix.check_version(">=7.0").unwrap());
        version.assert_async().await;
        authentication.assert_async().await;
        request.assert_async().await;
    }

    #[tokio::test]
    async fn password_login_and_explicit_logout_succeed() {
        let mut server = Server::new_async().await;
        let url = server.url();

        let version = server
            .mock("POST", "/api_jsonrpc.php")
            .match_body(Matcher::PartialJson(
                serde_json::json!({"method": "apiinfo.version"}),
            ))
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create_async()
            .await;
        let login = server
            .mock("POST", "/api_jsonrpc.php")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "method": "user.login",
                "params": {"username": "Admin", "password": "zabbix"}
            })))
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"session_token","id":1}"#)
            .create_async()
            .await;
        let logout = server
            .mock("POST", "/api_jsonrpc.php")
            .match_header("authorization", "Bearer session_token")
            .match_body(Matcher::PartialJson(
                serde_json::json!({"method": "user.logout"}),
            ))
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":true,"id":1}"#)
            .create_async()
            .await;

        let mut zabbix = ZabbixInstance::builder(&url)
            .build()
            .await
            .unwrap()
            .login(AuthType::UsernamePassword(
                "Admin".to_string(),
                "zabbix".to_string(),
            ))
            .await
            .unwrap();
        zabbix.logout().await.unwrap();

        version.assert_async().await;
        login.assert_async().await;
        logout.assert_async().await;
    }

    #[tokio::test]
    async fn api_errors_are_returned() {
        let mut server = Server::new_async().await;
        let url = server.url();

        server
            .mock("POST", "/api_jsonrpc.php")
            .match_body(Matcher::PartialJson(
                serde_json::json!({"method": "apiinfo.version"}),
            ))
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create_async()
            .await;
        server
            .mock("POST", "/api_jsonrpc.php")
            .match_body(Matcher::PartialJson(
                serde_json::json!({"method": "user.checkAuthentication"}),
            ))
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"valid","id":1}"#)
            .create_async()
            .await;
        server
            .mock("POST", "/api_jsonrpc.php")
            .match_body(Matcher::PartialJson(
                serde_json::json!({"method": "host.get"}),
            ))
            .with_status(200)
            .with_body(
                r#"{"jsonrpc":"2.0","error":{"message":"Invalid params","data":"Bad filter"},"id":1}"#,
            )
            .create_async()
            .await;

        let zabbix = ZabbixInstance::builder(&url)
            .build()
            .await
            .unwrap()
            .login(AuthType::Token("test_token".to_string()))
            .await
            .unwrap();
        let error = zabbix
            .zabbix_request("host.get", serde_json::json!({}))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ZabbixError::ApiError { message, data }
                if message == "Invalid params" && data == "Bad filter"
        ));
    }

    #[tokio::test]
    async fn http_errors_are_returned() {
        let mut server = Server::new_async().await;
        let url = server.url();

        let version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(503)
            .create_async()
            .await;

        let error = match ZabbixInstance::builder(&url).build().await {
            Ok(_) => panic!("expected the version request to fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            ZabbixError::Other(message) if message == "HTTP Error: 503 Service Unavailable"
        ));
        version.assert_async().await;
    }
}

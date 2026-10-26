//! A Rust library for interacting with the Zabbix API.
//!
//! This crate provides a convenient and idiomatic way to communicate with a Zabbix server,
//! handling authentication, version checking, and raw API requests.
//!
//! # Example
//!
//! ```no_run
//! # #[cfg(feature = "blocking")]
//! # {
//! use http_request_zabbix::{AuthType, ZabbixInstance};
//!
//! let zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
//!     .build()
//!     .unwrap()
//!     .login(AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string()))
//!     .unwrap();
//!
//! println!("Zabbix Version: {}", zabbix.get_version().unwrap());
//! # }
//! ```

#[cfg(feature = "blocking")]
use reqwest::blocking::Client;
#[cfg(feature = "blocking")]
use semver::{Version, VersionReq};
use serde_json::Value;
use thiserror::Error;
#[cfg(any(feature = "blocking", feature = "async"))]
use uuid::Uuid;

#[cfg(feature = "async")]
pub mod async_api;

/// Enum representing the type of authentication to use.
///
/// # Examples
///
/// ```no_run
/// use http_request_zabbix::AuthType;
///
/// let auth_type = AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string());
/// ```
///
/// ```no_run
/// use http_request_zabbix::AuthType;
///
/// let auth_type = AuthType::Token("817dc89d0ae1d347fbcacdd6c00f322d0ec0651a8df60115304216dc768205db".to_string());
/// ```
pub enum AuthType {
    /// Token authentication.
    /// Use it if you have a token from Zabbix. More info: https://www.zabbix.com/documentation/current/en/manual/web_interface/frontend_sections/users/api_tokens  
    Token(String),
    /// Username and password authentication.
    /// Use it if you have a username and password for Zabbix.
    UsernamePassword(String, String),
}

/// An enum representing the types of parameters that can be passed to the Zabbix API.
///
/// # Examples
///
/// ```no_run
/// use http_request_zabbix::ApiRequestParams;
///
/// let params_json = ApiRequestParams::from(serde_json::json!({"output": ["host", "name"], "limit": 1}));
/// let params_string = ApiRequestParams::from("{\"output\": [\"host\", \"name\"], \"limit\": 1}");
/// ```
pub enum ApiRequestParams {
    /// A raw pre-parsed JSON Value.
    Json(Value),
    /// A raw JSON string.
    String(String),
}

impl From<Value> for ApiRequestParams {
    fn from(v: Value) -> Self {
        ApiRequestParams::Json(v)
    }
}

impl From<&str> for ApiRequestParams {
    fn from(s: &str) -> Self {
        ApiRequestParams::String(s.to_string())
    }
}

impl From<String> for ApiRequestParams {
    fn from(s: String) -> Self {
        ApiRequestParams::String(s)
    }
}

/// Error type for Zabbix interactions.
///
/// # Errors
///
/// This method will return a `ZabbixError` if:
/// * The provided URL is invalid or unreachable (`ZabbixError::Network`).
/// * The server responds with invalid JSON (`ZabbixError::Json`).
/// * The server returns a version string that cannot be parsed by semantic versioning rules (`ZabbixError::VersionParse`).
/// * The server returns an API error (`ZabbixError::ApiError`). If future Zabbix versions return a different error format, this enum variant may need to be updated.
#[derive(Error, Debug)]
pub enum ZabbixError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Version parse error: {0}")]
    VersionParse(#[from] semver::Error),
    #[error("Zabbix API Error: {message} {data}")]
    ApiError { message: String, data: String },
    #[error("Unknown error: {0}")]
    Other(String),
}

#[cfg(any(feature = "blocking", feature = "async"))]
fn parse_params<P: Into<ApiRequestParams>>(params: P) -> Result<Value, ZabbixError> {
    match params.into() {
        ApiRequestParams::Json(value) => Ok(value),
        ApiRequestParams::String(value) => serde_json::from_str(&value).map_err(ZabbixError::from),
    }
}

#[cfg(any(feature = "blocking", feature = "async"))]
fn prepare_request(
    method: &str,
    params: Value,
    token: &str,
    need_auth_in_body: bool,
) -> (Value, Option<String>) {
    let mut payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": Uuid::new_v4().to_string()
    });

    let authorization = if token.is_empty() {
        None
    } else if need_auth_in_body {
        if let Some(object) = payload.as_object_mut() {
            object.insert("auth".to_string(), Value::String(token.to_string()));
        }
        None
    } else {
        Some(format!("Bearer {token}"))
    };

    (payload, authorization)
}

#[cfg(any(feature = "blocking", feature = "async"))]
fn parse_response(text: &str) -> Result<String, ZabbixError> {
    let json: Value = serde_json::from_str(text)?;

    if let Some(error) = json.get("error") {
        if error.is_object() {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Unknown error");
            let data = error.get("data").and_then(Value::as_str).unwrap_or("");
            return Err(ZabbixError::ApiError {
                message: message.to_string(),
                data: data.to_string(),
            });
        }
        return Err(ZabbixError::Other(error.to_string()));
    }

    if let Some(result) = json.get("result") {
        if let Some(value) = result.as_str() {
            return Ok(value.to_string());
        }
        return Ok(result.to_string());
    }

    Err(ZabbixError::Other("Unknown response format".to_string()))
}

/// Represents an active connection to a Zabbix server.
#[cfg(feature = "blocking")]
pub struct ZabbixInstance {
    id: String,
    url: String,
    token: String,
    request_client: Client,
    need_auth_in_body: bool,
    version: String,
    need_logout: bool,
}

#[cfg(feature = "blocking")]
impl ZabbixInstance {
    /// Creates a new `ZabbixInstanceBuilder` to configure the connection.
    ///
    /// The URL should be the base URL of the Zabbix server, without the `/api_jsonrpc.php` suffix.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_request_zabbix::ZabbixInstance;
    ///
    /// let builder = ZabbixInstance::builder("http://localhost/zabbix");
    /// ```
    pub fn builder(url: &str) -> ZabbixInstanceBuilder {
        ZabbixInstanceBuilder::new(url)
    }
}

/// A builder for creating a `ZabbixInstance`.
#[cfg(feature = "blocking")]
pub struct ZabbixInstanceBuilder {
    url: String,
    accept_invalid_certs: bool,
    client: Option<Client>,
    need_auth_in_body: bool,
    version: String,
}

#[cfg(feature = "blocking")]
impl ZabbixInstanceBuilder {
    /// Creates a new builder with the given Zabbix URL.
    ///
    /// The URL should be the base URL of the Zabbix server, without the `/api_jsonrpc.php` suffix.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_request_zabbix::ZabbixInstance;
    ///
    /// let builder = ZabbixInstance::builder("http://localhost/zabbix");
    /// ```
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            accept_invalid_certs: false,
            client: None,
            need_auth_in_body: false,
            version: "".to_string(),
        }
    }

    /// Configures whether the client should verify the server's TLS certificates.
    ///
    /// Setting this to `true` is dangerous and should only be used for testing
    /// or when using self-signed certificates in a trusted environment.
    ///
    /// # Examples
    ///
    /// ```
    /// use http_request_zabbix::ZabbixInstance;
    ///
    /// let builder = ZabbixInstance::builder("http://localhost/zabbix/api_jsonrpc.php")
    ///     .danger_accept_invalid_certs(true);
    /// ```
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Builds the `ZabbixInstance` by connecting to the server and verifying the API version.
    ///
    /// This method will make an initial unauthenticated request to the Zabbix server
    /// to determine its version (using `apiinfo.version`). This is required because
    /// Zabbix >= 6.4 changed the authentication flow (using Bearer tokens instead of
    /// passing auth in the request body).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use http_request_zabbix::ZabbixInstance;
    ///
    /// let zabbix_result = ZabbixInstance::builder("http://zabbix.example.com/api_jsonrpc.php")
    ///     .danger_accept_invalid_certs(true)
    ///     .build();
    ///     
    /// assert!(zabbix_result.is_ok());
    /// ```
    pub fn build(mut self) -> Result<Self, ZabbixError> {
        let client = Client::builder()
            .danger_accept_invalid_certs(self.accept_invalid_certs)
            .build()?;

        let v6_4_req = VersionReq::parse(">=6.4")?;

        let version_str_raw = ZabbixInstance::zabbix_raw_request(
            &client,
            &self.url,
            "apiinfo.version",
            serde_json::json!([]),
            "",
            false,
        )?;
        let version_str = version_str_raw.trim_matches('"');

        let current_v = Version::parse(version_str)?;

        self.need_auth_in_body = !(v6_4_req.matches(&current_v));

        self.client = Some(client);

        self.version = version_str.to_string();

        Ok(self)
    }

    /// Logs in to the Zabbix server using the provided authentication type.
    ///
    /// # Arguments
    ///
    /// * `auth_type` - The authentication type to use for logging in.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use http_request_zabbix::{ZabbixInstance, AuthType};
    ///
    /// let auth_type = AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string());
    ///
    /// let zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
    ///     .build()
    ///     .unwrap()
    ///     .login(auth_type)
    ///     .unwrap();
    /// ```
    ///
    /// ```no_run
    /// use http_request_zabbix::{ZabbixInstance, AuthType};
    ///
    /// let auth_type = AuthType::Token("817dc89d0ae1...".to_string());
    ///
    /// let zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
    ///     .build()
    ///     .unwrap()
    ///     .login(auth_type)
    ///     .unwrap();
    /// ```
    pub fn login(self, auth_type: AuthType) -> Result<ZabbixInstance, ZabbixError> {
        match auth_type {
            AuthType::Token(token) => self.login_with_token(token),
            AuthType::UsernamePassword(username, password) => {
                self.login_with_username_password(username, password)
            }
        }
    }

    fn login_with_token(self, token: String) -> Result<ZabbixInstance, ZabbixError> {
        let client = self.client.ok_or_else(|| {
            ZabbixError::Other("Client not initialized. Did you call build()?".to_string())
        })?;

        match ZabbixInstance::zabbix_raw_request(
            &client,
            &self.url,
            "user.checkAuthentication",
            serde_json::json!({"token": token}),
            "",
            self.need_auth_in_body,
        ) {
            Ok(_) => Ok(ZabbixInstance {
                id: Uuid::new_v4().to_string(),
                need_auth_in_body: self.need_auth_in_body,
                token,
                request_client: client,
                url: self.url,
                version: self.version,
                need_logout: false,
            }),
            Err(e) => Err(ZabbixError::ApiError {
                message: "Invalid token".to_string(),
                data: e.to_string(),
            }),
        }
    }

    fn login_with_username_password(
        self,
        username: String,
        password: String,
    ) -> Result<ZabbixInstance, ZabbixError> {
        let v5_2 = Version::parse("5.2.0")?;
        let current_v = Version::parse(&self.version)?;
        let user_param = if current_v <= v5_2 {
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
            serde_json::json!({user_param: username, "password": password}),
            "",
            self.need_auth_in_body,
        )?;

        Ok(ZabbixInstance {
            id: Uuid::new_v4().to_string(),
            need_auth_in_body: self.need_auth_in_body,
            token,
            request_client: client,
            url: self.url,
            version: self.version,
            need_logout: true,
        })
    }
}

#[cfg(feature = "blocking")]
impl ZabbixInstance {
    /// Returns the internally generated UUID for this instance.
    ///
    /// You can use it to identify the instance in your logs.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use http_request_zabbix::{ZabbixInstance, AuthType};
    ///
    /// let zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
    ///     .build()
    ///     .unwrap()
    ///     .login(AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string()))
    ///     .unwrap();
    /// println!("Instance ID: {}", zabbix.id());
    /// ```
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the Zabbix API URL this instance connects to (without the `/api_jsonrpc.php` suffix).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use http_request_zabbix::{ZabbixInstance, AuthType};
    ///
    /// let zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
    ///     .build()
    ///     .unwrap()
    ///     .login(AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string()))
    ///     .unwrap();
    /// println!("Instance URL: {}", zabbix.url());
    /// ```
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Logs out of the Zabbix server and invalidates the current token.
    ///
    /// Call automatically when the instance is dropped.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use http_request_zabbix::{ZabbixInstance, AuthType};
    ///
    /// let mut zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
    ///     .build()
    ///     .unwrap()
    ///     .login(AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string()))
    ///     .unwrap();
    /// zabbix.logout().unwrap();
    /// ```
    pub fn logout(&mut self) -> Result<&mut Self, ZabbixError> {
        if !self.need_logout {
            return Ok(self);
        }

        match Self::zabbix_raw_request(
            &self.request_client,
            &self.url,
            "user.logout",
            serde_json::json!([]),
            self.token.as_ref(),
            self.need_auth_in_body,
        ) {
            Ok(_) => {
                self.token = "".to_string();
                self.need_logout = false;
                Ok(self)
            }
            Err(e) => Err(e),
        }
    }

    /// Retrieves the Zabbix server API version.
    ///
    /// Use this method to check the version instead of directly calling `zabbix_request` with `apiinfo.version`,
    /// because this method requires no authentication.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use http_request_zabbix::{ZabbixInstance, AuthType};
    ///
    /// let zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
    ///     .build()
    ///     .unwrap()
    ///     .login(AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string()))
    ///     .unwrap();
    /// println!("Zabbix Version: {}", zabbix.get_version().unwrap());
    /// ```
    pub fn get_version(&self) -> Result<String, ZabbixError> {
        let version_str = Self::zabbix_raw_request(
            &self.request_client,
            &self.url,
            "apiinfo.version",
            serde_json::json!([]),
            "",
            false,
        )?;

        Ok(version_str)
    }

    /// Checks if the connected Zabbix server's version matches a semantic version requirement.
    /// Example requirement: `>=6.4, <7.0`
    ///
    /// You can use this method to quickly check if the connected Zabbix server's version satisfies your requirements.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use http_request_zabbix::{ZabbixInstance, AuthType};
    ///
    /// let zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
    ///     .build()
    ///     .unwrap()
    ///     .login(AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string()))
    ///     .unwrap();
    /// println!("Is >= 6.4: {}", zabbix.check_version(">=6.4").unwrap());
    /// ```
    pub fn check_version(&self, version_req: &str) -> Result<bool, ZabbixError> {
        let version_req = VersionReq::parse(version_req)?;
        let current_v = Version::parse(&self.version)?;

        Ok(version_req.matches(&current_v))
    }

    /// Makes a raw JSON-RPC request to the Zabbix API.
    ///
    /// `params` can be either a `serde_json::Value` (like `json!({...})`), a string slice `&str`, or a `String`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use http_request_zabbix::{ApiRequestParams, AuthType, ZabbixInstance};
    ///
    /// let zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix").build().unwrap()
    ///     .login(AuthType::UsernamePassword("Admin".to_string(), "zabbix".to_string())).unwrap();
    ///
    /// let params_json = ApiRequestParams::from(serde_json::json!({"output": ["host", "name"], "limit": 1}));
    /// let params_string = ApiRequestParams::from("{\"output\": [\"host\", \"name\"], \"limit\": 1}");
    ///
    /// let result_json = zabbix.zabbix_request("host.get", params_json).unwrap();
    /// let result_string = zabbix.zabbix_request("host.get", params_string).unwrap();
    /// ```
    pub fn zabbix_request<P: Into<ApiRequestParams>>(
        &self,
        method: &str,
        params: P,
    ) -> Result<String, ZabbixError> {
        let params_val = parse_params(params)?;

        Self::zabbix_raw_request(
            &self.request_client,
            &self.url,
            method,
            params_val,
            &self.token,
            self.need_auth_in_body,
        )
    }

    fn zabbix_raw_request(
        client: &Client,
        url: &str,
        method: &str,
        params: Value,
        token: &str,
        need_auth_in_body: bool,
    ) -> Result<String, ZabbixError> {
        let (payload, authorization) = prepare_request(method, params, token, need_auth_in_body);

        let mut request_builder = client
            .post(format!("{}/api_jsonrpc.php", url))
            .header("Content-Type", "application/json-rpc");

        if let Some(value) = authorization {
            request_builder = request_builder.header("Authorization", value);
        }

        let response = request_builder.json(&payload).send()?;

        if !response.status().is_success() {
            return Err(ZabbixError::Other(format!(
                "HTTP Error: {}",
                response.status()
            )));
        }

        parse_response(&response.text()?)
    }
}

#[cfg(feature = "blocking")]
impl Drop for ZabbixInstance {
    fn drop(&mut self) {
        if self.need_logout {
            self.logout().ok();
        }
    }
}

#[cfg(all(test, feature = "blocking"))]
mod tests {
    use super::*;
    use mockito::Server;

    #[test]
    fn test_login_with_token_success() {
        let mut server = Server::new();
        let url = server.url();

        let mock_version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create();

        let mock_auth = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"dummy_token","id":1}"#)
            .create();

        let builder = ZabbixInstanceBuilder::new(&url).build().unwrap();
        let result = builder.login(AuthType::Token("test_token".to_string()));

        assert!(result.is_ok());
        mock_version.assert();
        mock_auth.assert();
    }

    #[test]
    fn test_login_with_password_success() {
        let mut server = Server::new();
        let url = server.url();

        let mock_version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create();

        let mock_auth = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"dummy_token","id":1}"#)
            .create();

        let builder = ZabbixInstanceBuilder::new(&url).build().unwrap();
        let result = builder.login(AuthType::UsernamePassword(
            "Admin".to_string(),
            "zabbix".to_string(),
        ));

        assert!(result.is_ok());
        mock_version.assert();
        mock_auth.assert();
    }

    #[test]
    fn test_login_with_password_failure() {
        let mut server = Server::new();
        let url = server.url();

        let mock_version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create();

        let mock_auth = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(401)
            .with_body(r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params","data":"Invalid username or password"},"id":1}"#)
            .create();

        let builder = ZabbixInstanceBuilder::new(&url).build().unwrap();
        let result = builder.login(AuthType::UsernamePassword(
            "Admin".to_string(),
            "zabbix".to_string(),
        ));

        assert!(result.is_err());
        mock_version.assert();
        mock_auth.assert();
    }

    #[test]
    fn test_login_with_token_failure() {
        let mut server = Server::new();
        let url = server.url();

        let mock_version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create();

        let mock_auth = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params","data":"Token is invalid"},"id":1}"#)
            .create();

        let builder = ZabbixInstanceBuilder::new(&url).build().unwrap();
        let result = builder.login(AuthType::Token("test_token".to_string()));

        assert!(result.is_err());
        mock_version.assert();
        mock_auth.assert();
    }

    #[test]
    fn test_request_json_success() {
        let mut server = Server::new();
        let url = server.url();

        let mock_version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create();

        let mock_auth = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"dummy_token","id":1}"#)
            .create();

        let mock_request = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"dummy_result","id":1}"#)
            .create();

        let builder = ZabbixInstanceBuilder::new(&url).build().unwrap();
        let result = builder.login(AuthType::Token("test_token".to_string()));
        let host_get = result.unwrap().zabbix_request(
            "host.get",
            serde_json::json!({"output": ["host", "name"], "limit": 1}),
        );

        assert!(host_get.is_ok());
        mock_version.assert();
        mock_auth.assert();
        mock_request.assert();
    }

    #[test]
    fn test_request_json_failure() {
        let mut server = Server::new();
        let url = server.url();

        let mock_version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create();

        let mock_auth = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"dummy_token","id":1}"#)
            .create();

        let mock_request = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params","data":"BlahBlahBlah"},"id":1}"#)
            .create();

        let builder = ZabbixInstanceBuilder::new(&url).build().unwrap();
        let result = builder.login(AuthType::Token("test_token".to_string()));
        let host_get = result.unwrap().zabbix_request(
            "host.get",
            serde_json::json!({"output": ["host", "name"], "limit": 1}),
        );

        assert!(host_get.is_err());
        mock_version.assert();
        mock_auth.assert();
        mock_request.assert();
    }

    #[test]
    fn test_request_string_success() {
        let mut server = Server::new();
        let url = server.url();

        let mock_version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create();

        let mock_auth = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"dummy_token","id":1}"#)
            .create();

        let mock_request = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"dummy_result","id":1}"#)
            .create();

        let builder = ZabbixInstanceBuilder::new(&url).build().unwrap();
        let result = builder.login(AuthType::Token("test_token".to_string()));
        let host_get = result
            .unwrap()
            .zabbix_request("host.get", r#"{"output": ["host", "name"], "limit": 1}"#);

        assert!(host_get.is_ok());
        mock_version.assert();
        mock_auth.assert();
        mock_request.assert();
    }

    #[test]
    fn test_request_string_failure() {
        let mut server = Server::new();
        let url = server.url();

        let mock_version = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"7.0.0","id":1}"#)
            .create();

        let mock_auth = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","result":"dummy_token","id":1}"#)
            .create();

        let mock_request = server
            .mock("POST", "/api_jsonrpc.php")
            .with_status(200)
            .with_body(r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params","data":"BlahBlahBlah"},"id":1}"#)
            .create();

        let builder = ZabbixInstanceBuilder::new(&url).build().unwrap();
        let result = builder.login(AuthType::Token("test_token".to_string()));
        let host_get = result
            .unwrap()
            .zabbix_request("host.get", r#"{"output": ["host", "name"], "limit": 1}"#);

        assert!(host_get.is_err());
        mock_version.assert();
        mock_auth.assert();
        mock_request.assert();
    }
}

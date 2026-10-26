use http_request_zabbix::{AuthType, async_api::ZabbixInstance};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut zabbix = ZabbixInstance::builder("http://zabbix.example.com/zabbix")
        .danger_accept_invalid_certs(true)
        .build()
        .await?
        .login(AuthType::UsernamePassword(
            "Admin".to_string(),
            "zabbix".to_string(),
        ))
        .await?;

    println!("Zabbix Version: {}", zabbix.get_version().await?);

    let hosts = zabbix
        .zabbix_request(
            "host.get",
            serde_json::json!({"output": ["host", "name"], "limit": 1}),
        )
        .await?;
    println!("Hosts: {hosts}");

    zabbix.logout().await?;
    Ok(())
}

#[macro_use]
extern crate log;

const KEY_ID: &str = include_str!("../../creds/intersight-cgascoig-20200423.keyid.txt");
const SECRET_KEY: &[u8] = include_bytes!("../../creds/intersight-cgascoig-20200423.pem");

// const KEY_ID: &str = include_str!("../../creds/intersight-cgascoig-v3-20220329.keyid.txt");
// const SECRET_KEY: &[u8] = include_bytes!("../../creds/intersight-cgascoig-v3-20220329.pem");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    info!("starting up");
    info!("Using key_id {}", KEY_ID);

    let client = intersight_api::Client::from_key_bytes(KEY_ID, SECRET_KEY, None)?;

    // let response = client
    //     .post(
    //         "api/v1/ntp/Policies",
    //         serde_json::json!({
    //             "Name": "cg-rust-test",
    //             "Enabled": true,
    //             "Organization": {
    //                 "ClassId":"mo.MoRef",
    //                 "ObjectType": "organization.Organization",
    //                 "Selector": "Name eq \'default\'"
    //             },
    //             "NtpServers": ["1.1.1.1"],
    //         }),
    //     )
    //     .await?;

    let response = client.get("api/v1/ntp/Policies").await?;

    if let serde_json::Value::Array(ntp_policies) = &response["Results"] {
        for ntp_pol in ntp_policies {
            println!("{}", ntp_pol["Name"])
        }
    }

    Ok(())
}

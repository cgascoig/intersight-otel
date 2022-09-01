use opentelemetry::metrics;
use opentelemetry::sdk::export;
use opentelemetry::sdk::metrics::PushController;

#[macro_use]
extern crate log;

mod intersight_poller;

const KEY_ID: &str = include_str!("../../creds/intersight-cgascoig-20200423.keyid.txt");
const SECRET_KEY: &[u8] = include_bytes!("../../creds/intersight-cgascoig-20200423.pem");

// const KEY_ID: &str = include_str!("../../creds/intersight-cgascoig-v3-20220329.keyid.txt");
// const SECRET_KEY: &[u8] = include_bytes!("../../creds/intersight-cgascoig-v3-20220329.pem");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("starting up");
    info!("Using key_id {}", KEY_ID);

    let _ctrl = init_metrics_stdout()?;

    let client = intersight_api::Client::from_key_bytes(KEY_ID, SECRET_KEY, None)?;

    let poller_handle = tokio::spawn(async move {
        intersight_poller::start_poller(&client).await;
    });

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

    // let response = client.get("api/v1/ntp/Policies").await?;

    // if let serde_json::Value::Array(ntp_policies) = &response["Results"] {
    //     for ntp_pol in ntp_policies {
    //         println!("{}", ntp_pol["Name"])
    //     }
    // }

    poller_handle.await?;

    Ok(())
}

// fn init_metrics_otlp() -> metrics::Result<PushController> {
//     let export_config = ExportConfig {
//         endpoint: "http://localhost:4317".to_string(),
//         ..ExportConfig::default()
//     };
//     opentelemetry_otlp::new_pipeline()
//         .metrics(tokio::spawn, opentelemetry::util::tokio_interval_stream)
//         .with_exporter(
//             opentelemetry_otlp::new_exporter()
//                 .tonic()
//                 .with_export_config(export_config),
//         )
//         .build()
// }

use futures_util::{Stream, StreamExt as _};
use std::time::Duration;

// Skip first immediate tick from tokio, not needed for async_std.
fn delayed_interval(duration: Duration) -> impl Stream<Item = tokio::time::Instant> {
    opentelemetry::util::tokio_interval_stream(duration).skip(1)
}

fn init_metrics_stdout() -> metrics::Result<PushController> {
    let exporter = export::metrics::stdout(tokio::spawn, delayed_interval)
        .with_period(Duration::from_secs(5))
        .init();

    Ok(exporter)
}

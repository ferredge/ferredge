//! MQTT leg of the harness: connects to the host mosquitto, checks the negotiated
//! session, and publishes.

use alloc::{string::ToString, vec, vec::Vec};

use ferredge_core::prelude::*;
use ferredge_proto_mqtt::{MqttDriver, MqttListenerStatus};
use ferredge_runtime_embassy::{EmbassyNet, EmbassyRuntime};

use super::HOST_ADDR;

pub async fn run(runtime: &EmbassyRuntime, net: EmbassyNet) {
    let driver = MqttDriver::with_stack(
        Device {
            id: "harness-mqtt".to_string(),
            name: "mosquitto".to_string(),
            status: DeviceStatus::Online,
            endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
                broker: alloc::format!("{HOST_ADDR}:41883"),
                client_id: "qemu-harness".to_string(),
                auth: None,
                tls: None,
                keepalive_secs: Some(30),
                clean_start: true,
                session_expiry_secs: None,
                topic_prefix: None,
                connect_properties: MqttConnectProperties::default(),
                will: None,
                reconnect: BrokerReconnectConfig::default(),
                supported_versions: vec![MqttProtocolVersion::V5_0],
            }),
            metadata: None,
            max_connections: Some(4),
            resources: Map::default(),
            message_endpoints: Vec::new(),
        },
        runtime.clone(),
        net,
    );

    log::debug!("connecting to mosquitto at {HOST_ADDR}:41883");
    driver
        .start()
        .await
        .expect("mosquitto should accept the connection");
    let connack = driver
        .negotiated_connack_async()
        .await
        .expect("connack query should succeed")
        .expect("broker should have sent a CONNACK");
    log::debug!("mosquitto CONNACK {connack:?}");
    // `start()` only opens the session; the background listener is a separate
    // subsystem (`start_listening`) that this test doesn't exercise.
    assert_eq!(
        driver.listener_status_async().await,
        Ok(MqttListenerStatus::Stopped)
    );

    let publish = driver
        .native_packet_request(Command {
            id: "harness-publish-1".to_string(),
            source_device_id: None,
            target_device_id: "harness-mqtt".to_string(),
            intent: Intent::Send {
                channel: BrokerAddress {
                    name: "ferredge/harness".to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                payload: PayloadValue::from(&b"hello from qemu"[..]).into_owned(),
                options: BrokerMessageOptions::default(),
            },
            correlation: None,
        })
        .expect("publish request should convert");
    log::debug!("publishing \"hello from qemu\" to ferredge/harness");
    driver
        .publish(publish)
        .await
        .expect("publish should succeed");

    driver.stop().await.expect("mqtt stop should succeed");
}

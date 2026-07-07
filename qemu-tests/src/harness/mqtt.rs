//! MQTT leg of the harness: two drivers against the host mosquitto — a subscriber
//! with a live background listener and a publisher — plus the negotiated-session
//! checks. The subscriber must receive the publisher's message through the broker.

use alloc::{rc::Rc, string::ToString, vec, vec::Vec};
use core::cell::RefCell;

use ferredge_core::prelude::*;
use ferredge_proto_mqtt::{MqttDriver, MqttListenerStatus};
use ferredge_runtime_embassy::{EmbassyNet, EmbassyRuntime};

use super::HOST_ADDR;

const TOPIC: &str = "ferredge/harness";
const PAYLOAD: &[u8] = b"hello from qemu";

/// Collects every event the driver hands to the sink.
#[derive(Clone, Default)]
struct CollectSink(Rc<RefCell<Vec<RoutedEvent<'static>>>>);

impl EventSink for CollectSink {
    type Event = RoutedEvent<'static>;
    type Error = core::convert::Infallible;

    fn handle(&mut self, event: Self::Event) -> Result<(), Self::Error> {
        self.0.borrow_mut().push(event);
        Ok(())
    }
}

fn mqtt_device(id: &str, client_id: &str) -> Device<ferredge_proto_mqtt::MqttResourceAttributes> {
    Device {
        id: id.to_string(),
        name: "mosquitto".to_string(),
        status: DeviceStatus::Online,
        endpoint: DeviceEndpoint::mqtt(MqttEndpointConfig {
            broker: alloc::format!("{HOST_ADDR}:41883"),
            client_id: client_id.to_string(),
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
    }
}

pub async fn run(runtime: &EmbassyRuntime, net: EmbassyNet) {
    let subscriber = MqttDriver::with_stack(
        mqtt_device("harness-mqtt-sub", "qemu-harness-sub"),
        runtime.clone(),
        net.clone(),
    );
    let publisher = MqttDriver::with_stack(
        mqtt_device("harness-mqtt-pub", "qemu-harness-pub"),
        runtime.clone(),
        net,
    );

    log::debug!("connecting subscriber to mosquitto at {HOST_ADDR}:41883");
    subscriber
        .start()
        .await
        .expect("mosquitto should accept the subscriber connection");
    let connack = subscriber
        .negotiated_connack_async()
        .await
        .expect("connack query should succeed")
        .expect("broker should have sent a CONNACK");
    log::debug!("mosquitto CONNACK {connack:?}");
    // `start()` only opens the session; the background listener is a separate
    // subsystem (`start_listening`) exercised below.
    assert_eq!(
        subscriber.listener_status_async().await,
        Ok(MqttListenerStatus::Stopped)
    );

    let received = CollectSink::default();
    subscriber
        .start_listening(received.clone())
        .await
        .expect("start_listening should succeed");
    assert_eq!(
        subscriber.listener_status_async().await,
        Ok(MqttListenerStatus::Running)
    );

    let subscribe = subscriber
        .native_packet_request(Command {
            id: "harness-subscribe-1".to_string(),
            source_device_id: None,
            target_device_id: "harness-mqtt-sub".to_string(),
            intent: Intent::Subscribe {
                channel: BrokerAddress {
                    name: TOPIC.to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                options: BrokerSubscriptionOptions::default(),
            },
            correlation: None,
        })
        .expect("subscribe request should convert");
    log::debug!("subscribing to {TOPIC}");
    subscriber
        .subscribe(subscribe, received.clone())
        .await
        .expect("subscribe should succeed");

    log::debug!("connecting publisher to mosquitto at {HOST_ADDR}:41883");
    publisher
        .start()
        .await
        .expect("mosquitto should accept the publisher connection");
    let publish = publisher
        .native_packet_request(Command {
            id: "harness-publish-1".to_string(),
            source_device_id: None,
            target_device_id: "harness-mqtt-pub".to_string(),
            intent: Intent::Send {
                channel: BrokerAddress {
                    name: TOPIC.to_string(),
                    kind: Some(BrokerChannelKind::Topic),
                },
                payload: PayloadValue::from(PAYLOAD).into_owned(),
                options: BrokerMessageOptions::default(),
            },
            correlation: None,
        })
        .expect("publish request should convert");
    log::debug!("publishing {PAYLOAD:?} to {TOPIC}");
    publisher
        .publish(publish)
        .await
        .expect("publish should succeed");

    // The subscriber's background listener polls the session every 250ms; give the
    // broker roundtrip a generous window before declaring the message lost.
    let mut waited = core::time::Duration::ZERO;
    let poll = core::time::Duration::from_millis(50);
    while received.0.borrow().is_empty() {
        assert!(
            waited < core::time::Duration::from_secs(10),
            "subscriber did not receive the published message within 10s"
        );
        runtime.sleep(poll).await;
        waited += poll;
    }

    let events = received.0.borrow();
    let event = events
        .iter()
        .find(|event| matches!(&event.address, Address::Channel(topic) if topic == TOPIC))
        .expect("received event should target the subscribed topic");
    assert_eq!(
        event.payload,
        PayloadValue::Bytes(PAYLOAD.into()),
        "unexpected payload: {:?}",
        event.payload
    );
    drop(events);
    log::debug!("subscriber received the published message");

    publisher.stop().await.expect("publisher stop should succeed");
    subscriber
        .stop()
        .await
        .expect("subscriber stop should succeed");
    assert_eq!(
        subscriber.listener_status_async().await,
        Ok(MqttListenerStatus::Stopped)
    );
}

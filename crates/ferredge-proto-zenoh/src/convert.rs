extern crate alloc;

use alloc::{borrow::Cow, string::ToString, vec, vec::Vec};

use ferredge_bridge::{
    BridgeMessage, BridgeOutbound, BridgePayload, NativeOutbound, ProtocolEncoder,
};
use ferredge_core::prelude::*;

use crate::types::{
    ZenohCommandConversionError, ZenohCommandRef, ZenohNativePlan, ZenohPublishRequest,
    ZenohQueryRequest, ZenohRequest, ZenohResourceAttributes,
};

/// Bridge codec for Zenoh protocol.
pub struct ZenohBridgeCodec<'a> {
    command_ref: ZenohCommandRef<'a>,
}

impl<'a> ZenohBridgeCodec<'a> {
    /// Creates a codec bound to one Zenoh device definition.
    pub fn new(device: &'a Device<ZenohResourceAttributes>) -> Self {
        Self {
            command_ref: ZenohCommandRef { device },
        }
    }
}

impl ProtocolEncoder<BridgeOutbound, BridgeMessage<'static>, ZenohRequest>
    for ZenohBridgeCodec<'_>
{
    type Error = ZenohCommandConversionError;

    fn encode(&self, message: BridgeMessage<'static>) -> Result<ZenohRequest, Self::Error> {
        let BridgeMessage::Command(bridge) = message else {
            return Err(ZenohCommandConversionError::UnsupportedIntent(
                "only command messages are supported".to_string(),
            ));
        };

        match &bridge.operation {
            ferredge_bridge::BridgeOp::Messaging(op) => match op.action {
                ferredge_bridge::MessagingAction::Publish => {
                    let key_expr = match &bridge.route {
                        ferredge_bridge::BridgeRoute::Messaging { topic } => topic.to_string(),
                        _ => {
                            return Err(ZenohCommandConversionError::InvalidAddressType(
                                "expected messaging route".to_string(),
                            ))
                        }
                    };

                    let payload = bridge
                        .payload
                        .as_ref()
                        .map(payload_to_bytes)
                        .transpose()?
                        .unwrap_or_default();

                    Ok(ZenohRequest::Publish(ZenohPublishRequest {
                        command_id: bridge.id.into_owned(),
                        key_expr,
                        payload,
                    }))
                }
                _ => Err(ZenohCommandConversionError::UnsupportedIntent(
                    alloc::format!("messaging action {:?} not supported", op.action),
                )),
            },
            ferredge_bridge::BridgeOp::RequestResponse(op) => match op.action {
                ferredge_bridge::RequestResponseAction::Read => {
                    let key_expr = match &bridge.route {
                        ferredge_bridge::BridgeRoute::RequestResponse { resource, .. } => {
                            resource.to_string()
                        }
                        _ => {
                            return Err(ZenohCommandConversionError::InvalidAddressType(
                                "expected request/response route".to_string(),
                            ))
                        }
                    };

                    Ok(ZenohRequest::Query(ZenohQueryRequest {
                        command_id: bridge.id.into_owned(),
                        key_expr,
                        parameters: None,
                        timeout_ms: None,
                    }))
                }
                _ => Err(ZenohCommandConversionError::UnsupportedIntent(
                    alloc::format!("request/response action {:?} not supported", op.action),
                )),
            },
            _ => Err(ZenohCommandConversionError::UnsupportedIntent(
                alloc::format!("operation {:?} not supported", bridge.operation),
            )),
        }
    }
}

impl ProtocolEncoder<NativeOutbound, ZenohNativePlan<'static>, ZenohRequest>
    for ZenohBridgeCodec<'_>
{
    type Error = ZenohCommandConversionError;

    fn encode(&self, plan: ZenohNativePlan<'static>) -> Result<ZenohRequest, Self::Error> {
        match plan {
            ZenohNativePlan::Put {
                command_id,
                key_expr,
                payload,
            } => Ok(ZenohRequest::Publish(ZenohPublishRequest {
                command_id: command_id.into_owned(),
                key_expr: key_expr.into_owned(),
                payload: payload_to_bytes(&payload)?,
            })),
            ZenohNativePlan::Get {
                command_id,
                key_expr,
                parameters,
            } => Ok(ZenohRequest::Query(ZenohQueryRequest {
                command_id: command_id.into_owned(),
                key_expr: key_expr.into_owned(),
                parameters: parameters.map(|p| p.into_owned()),
                timeout_ms: None,
            })),
            ZenohNativePlan::Subscribe {
                command_id: _,
                key_expr: _,
            } => {
                // Subscribe is handled differently - not converted to a request
                // but managed by the driver's subscribe() method
                Err(ZenohCommandConversionError::UnsupportedIntent(
                    "Subscribe plan should be handled by PubSub::subscribe()".to_string(),
                ))
            }
            _ => Err(ZenohCommandConversionError::UnsupportedIntent(
                "only Put and Get plans supported for request encoding".to_string(),
            )),
        }
    }
}

fn payload_to_bytes(payload: &BridgePayload) -> Result<Vec<u8>, ZenohCommandConversionError> {
    match payload {
        BridgePayload::Binary(bytes) => Ok(bytes.to_vec()),
        BridgePayload::Text(value) => Ok(value.as_bytes().to_vec()),
        BridgePayload::Empty => Ok(vec![]),
        other => serde_json::to_vec(&PayloadValue::from(other.clone()))
            .map_err(|e| ZenohCommandConversionError::PayloadEncoding(e.to_string())),
    }
}

/// Convert a Command to a ZenohNativePlan (planner function).
pub fn plan_from_command(
    command: &Command,
    _device: &Device<ZenohResourceAttributes>,
) -> Result<ZenohNativePlan<'static>, ZenohCommandConversionError> {
    let command_id = Cow::Owned(command.id.clone());

    match &command.intent {
        Intent::Send {
            channel, payload, ..
        } => {
            let key_expr = match &channel.kind {
                Some(BrokerChannelKind::Topic) | None => Cow::Owned(channel.name.clone()),
                _ => {
                    return Err(ZenohCommandConversionError::InvalidAddressType(
                        "Zenoh only supports topic-like channels".to_string(),
                    ))
                }
            };

            let bridge_payload = BridgePayload::from(payload.clone());

            Ok(ZenohNativePlan::Put {
                command_id,
                key_expr,
                payload: bridge_payload,
            })
        }
        Intent::Subscribe { channel, .. } => {
            let key_expr = match &channel.kind {
                Some(BrokerChannelKind::Topic) | None => Cow::Owned(channel.name.clone()),
                _ => {
                    return Err(ZenohCommandConversionError::InvalidAddressType(
                        "Zenoh only supports topic-like channels".to_string(),
                    ))
                }
            };

            Ok(ZenohNativePlan::Subscribe {
                command_id,
                key_expr,
            })
        }
        Intent::Unsubscribe { channel } => {
            let key_expr = match &channel.kind {
                Some(BrokerChannelKind::Topic) | None => Cow::Owned(channel.name.clone()),
                _ => {
                    return Err(ZenohCommandConversionError::InvalidAddressType(
                        "Zenoh only supports topic-like channels".to_string(),
                    ))
                }
            };

            Ok(ZenohNativePlan::Unsubscribe {
                command_id,
                key_expr,
            })
        }
        Intent::Read { resource, .. } => {
            let key_expr = Cow::Owned(resource.clone());
            Ok(ZenohNativePlan::Get {
                command_id,
                key_expr,
                parameters: None,
            })
        }
        Intent::Write { resource, payload, .. } => {
            let key_expr = Cow::Owned(resource.clone());
            let bridge_payload = BridgePayload::from(payload.clone());

            Ok(ZenohNativePlan::Put {
                command_id,
                key_expr,
                payload: bridge_payload,
            })
        }
        Intent::Invoke { .. } => Err(ZenohCommandConversionError::UnsupportedIntent(
            "Invoke intent not yet supported for Zenoh".to_string(),
        )),
    }
}

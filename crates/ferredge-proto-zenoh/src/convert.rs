extern crate alloc;

use alloc::{borrow::Cow, string::ToString, vec, vec::Vec};

use ferredge_bridge::{
    BridgeMessage, BridgeOutbound, NativeOutbound, ProtocolEncoder,
};
use ferredge_core::prelude::*;

use crate::types::{
    ZenohCommandConversionError, ZenohCommandRef, ZenohNativePlan, ZenohPublishRequest,
    ZenohQueryRequest, ZenohRequest,
};

/// Bridge codec for Zenoh protocol.
pub struct ZenohBridgeCodec<'a> {
    #[allow(dead_code)]
    command_ref: ZenohCommandRef<'a>,
}

impl<'a> ZenohBridgeCodec<'a> {
    /// Creates a codec bound to one Zenoh device definition.
    pub fn new(device: &'a crate::ZenohDevice) -> Self {
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
                        Route::Messaging { topic } => topic.to_string(),
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
                        command_id: bridge.id.to_string(),
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
                        Route::RequestResponse { resource, .. } => {
                            resource.to_string()
                        }
                        _ => {
                            return Err(ZenohCommandConversionError::InvalidAddressType(
                                "expected request/response route".to_string(),
                            ))
                        }
                    };

                    Ok(ZenohRequest::Query(ZenohQueryRequest {
                        command_id: bridge.id.to_string(),
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

fn payload_to_bytes(payload: &PayloadValue) -> Result<Vec<u8>, ZenohCommandConversionError> {
    match payload {
        PayloadValue::Bytes(bytes) => Ok(bytes.to_vec()),
        PayloadValue::String(value) => Ok(value.as_bytes().to_vec()),
        PayloadValue::Null => Ok(vec![]),
        other => serde_json::to_vec(other)
            .map_err(|e| ZenohCommandConversionError::PayloadEncoding(e.to_string())),
    }
}

/// Convert a Command to a ZenohNativePlan (planner function).
pub fn plan_from_command(
    command: &Command,
    _device: &crate::ZenohDevice,
) -> Result<ZenohNativePlan<'static>, ZenohCommandConversionError> {
    let command_id = Cow::Owned(command.id.to_string());

    match &command.intent {
        Intent::Send {
            channel, payload, ..
        } => {
            let key_expr = match &channel.kind {
                Some(BrokerChannelKind::Topic) | None => Cow::Owned(channel.name.to_string()),
                _ => {
                    return Err(ZenohCommandConversionError::InvalidAddressType(
                        "Zenoh only supports topic-like channels".to_string(),
                    ))
                }
            };

            Ok(ZenohNativePlan::Put {
                command_id,
                key_expr,
                payload: payload.clone().into_owned(),
            })
        }
        Intent::Subscribe { channel, .. } => {
            let key_expr = match &channel.kind {
                Some(BrokerChannelKind::Topic) | None => Cow::Owned(channel.name.to_string()),
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
                Some(BrokerChannelKind::Topic) | None => Cow::Owned(channel.name.to_string()),
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
            let key_expr = Cow::Owned(resource.to_string());
            Ok(ZenohNativePlan::Get {
                command_id,
                key_expr,
                parameters: None,
            })
        }
        Intent::Write { resource, payload, .. } => {
            let key_expr = Cow::Owned(resource.to_string());

            Ok(ZenohNativePlan::Put {
                command_id,
                key_expr,
                payload: payload.clone().into_owned(),
            })
        }
        Intent::Invoke { operation, args, .. } => {
            // Map Invoke with "queryable" operation to DeclareQueryable
            if operation.as_str() == "queryable" {
                // Extract key_expr from args (args is a PayloadValue which can be a Map)
                let key_expr = args
                    .as_ref()
                    .and_then(|payload| match payload {
                        PayloadValue::Map(map) => {
                            // Find key_expr in the map
                            map.iter()
                                .find(|(k, _)| k.as_ref() == "key_expr")
                                .map(|(_, v)| v)
                        }
                        _ => None,
                    })
                    .and_then(|v| match v {
                        PayloadValue::String(s) => Some(Cow::Owned(s.to_string())),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        ZenohCommandConversionError::UnsupportedIntent(
                            "Invoke queryable requires 'key_expr' string argument in map".to_string(),
                        )
                    })?;

                Ok(ZenohNativePlan::DeclareQueryable {
                    command_id,
                    key_expr,
                })
            } else {
                Err(ZenohCommandConversionError::UnsupportedIntent(
                    alloc::format!("Invoke operation '{}' not supported for Zenoh", operation),
                ))
            }
        }
        _ => Err(ZenohCommandConversionError::UnsupportedIntent(
            "Intent not supported for Zenoh".to_string(),
        )),
    }
}

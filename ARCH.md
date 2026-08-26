# Architecture

This document describes the canonical routing and bridge message model
(issue #29) as implemented. It is a contributor reference: every type named
here exists in the code, at the path given.

## Overview

Workspace crates (`crates/`):

| Crate | Role |
|---|---|
| `ferredge-core` | Canonical message model, device model, driver capability traits, runtime/net/serial/sync adapter traits (feature-gated) |
| `ferredge-bridge` | Bridge intermediate representation (IR), semantic planners, plug-in planner/encoder/decoder traits |
| `ferredge-proto-mqtt` / `-http` / `-modbus` | Protocol drivers: endpoint configs, resource attributes, typed metadata views, planners, codecs |
| `ferredge-bridge-interop` | Cross-protocol bridging integration tests |
| `ferredge-runtime-std` / `-tokio` / `-async-std` / `-embassy` | Runtime adapters implementing the core runtime/net/serial traits |
| `ferredge-test-support` | Shared test helpers |

`ferredge-core` and `ferredge-bridge` are `no_std` + `alloc` (`std` is an
opt-in feature). All model types use `Cow`-based borrowing with explicit
lifetimes; nothing in the model layer requires a heap-owned copy up front.
The embassy runtime additionally enables core's `local` feature, which
relaxes `Send`/`Sync` bounds (`MaybeSend`/`MaybeSync` in `core/src/maybe.rs`)
for single-executor targets.

### Pipeline

```
outbound (per command):

  Command<'a>  (ferredge-core)
      |
      |  ProtocolPlanner<BridgeOutbound>            ProtocolPlanner<NativeOutbound>
      v                                                  |
  BridgeMessage<'a>  (ferredge-bridge IR)                |   fast path: skips the IR
      |                                                  v
      |  ProtocolEncoder<...>                    <Proto>NativePlan<'a>  (proto crate)
      v                                                  |
  native wire type (HttpRequest, MqttPacketRequest,      |  ProtocolEncoder<...>
  ModbusRequest, ...)                        <-----------+

inbound:

  native wire type --ProtocolDecoder--> Decoded<'a> view
      --> RoutedEvent<'a> / RoutedResult<'a>  (ferredge-core)
      --> BridgeMessage<'a> (Event/Result/Fault) via bridge planner helpers
```

Both outbound paths exist per protocol. The bridge IR path is used when a
message must cross protocols (MQTT device talking to a Modbus device); the
native path is a zero-overhead shortcut when the source command already
targets the same protocol.

## The canonical message model

One set of leaf types lives in `ferredge-core` and is re-exported by
`ferredge-bridge` (see `bridge/src/lib.rs`); the bridge defines only its
envelope and vocabulary on top. There are no duplicate leaf definitions.

| Type | Defined in | Role |
|---|---|---|
| `PayloadValue<'a>` | `core/src/command.rs` | Typed payload: scalars, `String`/`Bytes` (`Cow`, zero-copy), `List`/`Map` (`Vec`) |
| `Correlation<'a>`, `Address<'a>` | `core/src/command.rs` | Reply correlation; `Address` survives as the type of `Correlation.reply_to` |
| `Header<'a>`, `Headers<'a>` | `core/src/headers.rs` | Ordered protocol header/property pairs |
| `Route<'a>`, `AddressedAccessMeta` | `core/src/route.rs` | Lossless logical route: `RequestResponse` / `Messaging` / `AddressedAccess` |
| `ProtocolId`, `MetaValue<'a>`, `TransportMeta<'a>`, `META_KEY_HEADERS` | `core/src/meta.rs` | Open transport metadata (next section) |
| `IntoOwned` | `core/src/owned.rs` | Borrowed-to-owned conversion trait |
| `Command<'a>`, `Intent<'a>`, `CommandResult<'a>`, option structs | `core/src/command.rs` | Protocol-neutral command layer |
| `RoutedEvent<'a>`, `RoutedResult<'a>`, `RoutedMessage<'a>`, `EndpointRef` | `core/src/routed.rs` | Normalized inbound envelopes |
| `BridgeMessage<'a>` and friends | `bridge/src/message.rs` | Bridge IR envelope |
| `BridgeCapability`, `BridgeOp<'a>`, `BridgeFault` | `bridge/src/{capability,op,fault}.rs` | Bridge-level vocabulary |

### Command layer (core)

`Command<'a>` carries `id`, optional `source_device_id`, `target_device_id`,
an `Intent<'a>`, and `correlation: Option<Correlation<'a>>` — the single
source of correlation truth; nothing else in the pipeline re-derives it.

`Intent<'a>` covers `Read`/`Write`/`Invoke` (request/response style, with
`RequestOptions<'a>`) and `Send`/`Subscribe`/`Unsubscribe` (broker style,
with `BrokerMessageOptions<'a>` / `BrokerSubscriptionOptions<'a>` and
`BrokerAddress<'a>`). Every options struct ends with
`extensions: Option<TransportMeta<'a>>` — the open replacement for the old
closed per-protocol option enums. Protocol-specific option builders such as
`MqttMessageOptions` / `MqttSubscriptionOptions` live in `proto-mqtt/src/meta.rs`
and encode themselves into the capture via `into_transport_meta()`.

### Bridge IR

`BridgeMessage<'a>` (`bridge/src/message.rs`) is a four-way envelope:

- `Command(BridgeCommand<'a>)` — normalized outbound command: `capability`
  (`BridgeCapability`), `operation` (`BridgeOp<'a>`), optional payload,
  `route: Route<'a>`, optional `transport: TransportMeta<'a>`,
  `headers: Option<Headers<'a>>`, correlation.
- `Event(BridgeEvent<'a>)` — normalized inbound event with the same shape
  plus a source `EndpointRef`.
- `Result(BridgeResult<'a>)` — `{ meta: BridgeMessageMeta<'a>, outcome:
  BridgeOutcome<'a> }` where `BridgeOutcome` is `Progress { state }`,
  `Success { payload }`, or `Failure { state, payload, error, fault }`.
- `Fault(BridgeFaultMessage<'a>)` — a `BridgeFault` (category, protocol
  code, retryability, source protocol) with optional context.

`BridgeCapability` (Messaging / RequestResponse / RegisterAccess, plus
reserved variants) and `BridgeOp` (actions plus transport-neutral knobs
such as method, delivery guarantee, durable name) are the bridge's semantic
vocabulary; they never name a concrete protocol.

`bridge/src/planner.rs` provides the semantic planner helpers that build the
IR from core types: `command_to_request_response`, `command_to_messaging`,
`command_to_register_access`, and the inbound wrappers
`routed_event_to_bridge` / `routed_result_to_bridge`. Protocol planners
delegate to these rather than reimplementing the mapping.

### Plug-in traits

`bridge/src/traits.rs` defines the extension surface (signatures verbatim):

```rust
/// Marker for planning or encoding via the optional bridge IR.
pub struct BridgeOutbound;
/// Marker for planning or encoding directly through protocol-native semantics.
pub struct NativeOutbound;

pub trait ProtocolPlanner<TTarget> {
    type Error;
    type Planned<'a>;
    fn plan<'a>(&self, command: Command<'a>) -> Result<Self::Planned<'a>, Self::Error>;
}

pub trait ProtocolEncoder<TTarget, TPlanned, TNative> {
    type Error;
    fn encode(&self, planned: TPlanned) -> Result<TNative, Self::Error>;
}

pub trait ProtocolDecoder<TNative> {
    type Error;
    type Decoded<'a>
    where
        TNative: 'a;
    fn decode<'a>(&self, native: &'a TNative) -> Result<Self::Decoded<'a>, Self::Error>;
}
```

The GAT `Planned<'a>` lets one planner produce output whose lifetime tracks
each input command — this is what carries zero-copy through the plan step.

### Driver capability traits

`core/src/router.rs` defines the capabilities a driver may implement, each
generic over its native types: `RequestResponse` (execute one native
request), `PubSub` (publish / subscribe / unsubscribe), `EventSource`
(unsolicited inbound events into an `EventSink`), and `Lifecycle`
(start/stop). Drivers implement only the capabilities their protocol has.

## Transport metadata

`TransportMeta<'a>` (`core/src/meta.rs`) is an open, protocol-keyed capture:

```rust
pub struct TransportMeta<'a> {
    pub protocol: ProtocolId,
    pub entries: Vec<(Cow<'static, str>, MetaValue<'a>)>,
}
```

with `get`/`set`/`take` plus typed accessors (`get_str`, `get_u64`, ...).
`MetaValue<'a>` covers scalars, `Str`/`Bytes` Cows, lists, and `Pairs`
(name/value strings). `ProtocolId` is a `Cow<'static, str>` newtype with
`const` identities for the built-ins (`ProtocolId::MQTT`, `::HTTP`,
`::MODBUS`, `::COAP`); any external crate can mint its own const without
touching core.

Why this shape, and not the alternatives: a closed enum would force a core
edit (and a breaking release) for every new protocol, defeating the
extension goal. A generic parameter on the envelopes would monomorphize the
entire pipeline per protocol and make cross-protocol bridging — the whole
point — impossible to express in one message type. `dyn Any` would lose
`no_std`-friendly serialization, equality, and lossless pass-through.

### Typed views

Raw string-keyed captures are for transport; protocol crates own ergonomic
typed views over them, all following the same pattern (`HttpMeta<'a>` in
`proto-http/src/meta.rs`, `MqttMeta<'a>` in `proto-mqtt/src/meta.rs`):

- documented `pub const KEY_*: &str` capture keys,
- `into_transport_meta(self) -> TransportMeta<'a>` — encode by move, zero-copy,
- `from_transport_meta(meta: TransportMeta<'a>) -> Option<Self>` — decode by
  move (returns `None` if `meta.protocol` doesn't match),
- `view(meta: &'a TransportMeta<'a>) -> Option<Self>` — borrowing view
  without consuming the capture.

### Header flow (`META_KEY_HEADERS`)

Wire-level name/value headers (HTTP headers, MQTT v5 user properties) ride
the capture as `MetaValue::Pairs` under the well-known key
`META_KEY_HEADERS` when produced by drivers or option builders. The bridge
planner (`split_capture_headers` in `bridge/src/planner.rs`) splits that
entry out of the capture and promotes it to the envelope-level
`Headers<'a>` field, so bridge consumers see headers uniformly regardless
of source protocol, while the rest of the capture passes through losslessly.

## Zero-copy and ownership

Every model type is lifetimed; leaf strings and bytes are `Cow`s. The
discipline:

- **Through the pipeline, move — don't clone.** `plan` consumes
  `Command<'a>` and produces `Planned<'a>`; encoders consume the plan. Cows
  are moved field-to-field, so a payload borrowed from an inbound buffer
  stays borrowed all the way to the outbound encoder.
- **`IntoOwned` (`core/src/owned.rs`)** converts any borrowed model value to
  its `'static` counterpart. It is implemented for the Cows, `Option`,
  `Vec`, and tuples, and every model type wires it up (plus an inherent
  `into_owned()` convenience). `as_borrowed()` on the leaf types gives the
  reverse: a cheap re-borrowing view.
- **Sanctioned owned boundaries** — the only places `.into_owned()` is
  expected, exactly once each:
  1. `PubSub::subscribe` (`core/src/router.rs`) requires
     `EventSink<Event = RoutedEvent<'static>>`, deliberately: a subscription
     outlives any borrowed inbound frame.
  2. Queue / storage crossings — anything that outlives the current call
     stack (persistence, reconnect replay queues, cross-task channels).

Adding an `.into_owned()` anywhere else is a copy bug, not a convenience.

## Devices and endpoints

The device model (`core/src/device.rs`) is open on both axes:

```rust
pub trait DeviceEndpoint: Clone + core::fmt::Debug + Serialize + for<'de> Deserialize<'de> {
    fn protocol(&self) -> ProtocolId;
}

pub trait DeviceResourceAttributes:
    Clone + core::fmt::Debug + for<'de> Deserialize<'de> + Serialize
{
}

pub struct Device<T: DeviceResourceAttributes, E: DeviceEndpoint> { /* id, name, status,
    endpoint: E, resources: Map<String, DeviceResource<T>>, message_endpoints, ... */ }
```

Endpoint configs live in their protocol crates, each with a device alias:

- `HttpEndpointConfig` / `HttpDevice` (`proto-http/src/endpoint.rs`)
- `MqttEndpointConfig` / `MqttDevice` (`proto-mqtt/src/endpoint.rs`) —
  includes will, CONNECT/CONNACK properties, reconnect policy, versions
- `ModbusEndpoint` (enum over Tcp / RtuOverTcp / Rtu / Ascii / Udp) /
  `ModbusDevice` (`proto-modbus/src/endpoint.rs`)

Resource attributes likewise: `HttpResourceAttributes`,
`ModbusResourceAttributes`, `MqttResourceAttributes` each implement the
`DeviceResourceAttributes` marker in their crate.

Core keeps only protocol-neutral shared config: `AuthConfig`, `TlsConfig`,
`SerialPortConfig` + serial enums, `BrokerReconnectConfig`,
`BackoffStrategy` (all in `core/src/device.rs`).

The Modbus driver is additionally generic over `ModbusTransport`
(`proto-modbus/src/transport.rs`), so a device carries only the transport
stack it actually needs.

## Adding a protocol (worked example: CoAP)

A new protocol is a new crate. **Zero edits to `ferredge-core` or
`ferredge-bridge` are required** — every extension point below is a trait
impl or a const in your own crate. (CoAP happens to have a pre-minted
`ProtocolId::COAP`; a protocol core has never heard of simply defines
`const MYPROTO: ProtocolId = ProtocolId(Cow::Borrowed("myproto"));`.)

The snippets are illustrative (bodies elided) but the trait signatures are
copied from the source.

### 1. Endpoint config

```rust
// ferredge-proto-coap/src/endpoint.rs
pub type CoapDevice = Device<CoapResourceAttributes, CoapEndpointConfig>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoapEndpointConfig {
    pub host: String,
    pub port: u16,
    pub dtls: Option<TlsConfig>, // reuse core's neutral config where it fits
}

impl DeviceEndpoint for CoapEndpointConfig {
    fn protocol(&self) -> ProtocolId {
        ProtocolId::COAP
    }
}
```

### 2. Resource attributes

```rust
// ferredge-proto-coap/src/attributes.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoapResourceAttributes {
    pub path: String,
    pub confirmable: bool,
}

impl DeviceResourceAttributes for CoapResourceAttributes {}
```

### 3. Typed metadata view

Document each capture key as a `pub const`, and implement the three-method
view pattern over `TransportMeta` (see `MqttMeta`/`HttpMeta` for the
reference implementations):

```rust
// ferredge-proto-coap/src/meta.rs
/// Capture key for the CoAP message type (`"con"` / `"non"`).
pub const KEY_MESSAGE_TYPE: &str = "message-type";
/// Capture key for the CoAP Observe option sequence number.
pub const KEY_OBSERVE: &str = "observe";
/// Capture key for the CoAP token.
pub const KEY_TOKEN: &str = "token";

#[derive(Debug, Clone, Default)]
pub struct CoapMeta<'a> {
    pub message_type: Option<Cow<'a, str>>,
    pub observe: Option<u64>,
    pub token: Option<Cow<'a, [u8]>>,
}

impl<'a> CoapMeta<'a> {
    /// Encode by move — zero-copy.
    pub fn into_transport_meta(self) -> TransportMeta<'a> {
        let mut meta = TransportMeta::new(ProtocolId::COAP);
        if let Some(token) = self.token {
            meta.set(KEY_TOKEN, MetaValue::Bytes(token));
        }
        // ... remaining fields ...
        meta
    }

    /// Decode by move; `None` if the capture belongs to another protocol.
    pub fn from_transport_meta(meta: TransportMeta<'a>) -> Option<Self> {
        if meta.protocol != ProtocolId::COAP {
            return None;
        }
        // drain meta.entries into typed fields ...
        unimplemented!()
    }

    /// Borrowing view without consuming the capture.
    pub fn view(meta: &'a TransportMeta<'a>) -> Option<CoapMeta<'a>> {
        if meta.protocol != ProtocolId::COAP {
            return None;
        }
        Some(CoapMeta {
            token: meta.get_bytes(KEY_TOKEN).map(Cow::Borrowed),
            // ...
        })
    }
}
```

If CoAP carries name/value properties that should surface as envelope
headers, store them as `MetaValue::Pairs` under `META_KEY_HEADERS`; the
bridge planner promotes them automatically.

### 4. Planner, encoder, decoder

CoAP is request/response shaped, so the bridge planner delegates to the
existing semantic helper — no new IR mapping code:

```rust
pub struct CoapCommandPlanner;

impl ProtocolPlanner<BridgeOutbound> for CoapCommandPlanner {
    type Error = BridgePlannerError;
    type Planned<'a> = BridgeMessage<'a>;

    fn plan<'a>(&self, command: Command<'a>) -> Result<BridgeMessage<'a>, Self::Error> {
        ferredge_bridge::planner::command_to_request_response(command)
    }
}
```

Optionally add the fast path that skips the IR when no protocol translation
is needed, producing your own lifetimed plan type:

```rust
impl ProtocolPlanner<NativeOutbound> for CoapCommandPlanner {
    type Error = CoapCommandConversionError;
    type Planned<'a> = CoapNativePlan<'a>;

    fn plan<'a>(&self, command: Command<'a>) -> Result<CoapNativePlan<'a>, Self::Error> {
        // destructure command.intent, move Cows into the plan — no clones
        unimplemented!()
    }
}
```

Encode plans into your wire type (the encoder typically borrows the device
definition to resolve resources):

```rust
impl<'p> ProtocolEncoder<NativeOutbound, CoapNativePlan<'p>, CoapRequest>
    for CoapRequestEncoder<'_>
{
    type Error = CoapCommandConversionError;

    fn encode(&self, planned: CoapNativePlan<'p>) -> Result<CoapRequest, Self::Error> {
        unimplemented!()
    }
}
```

Decode inbound native values into a borrowed semantic view, then convert
that view into `RoutedEvent<'a>` / `RoutedResult<'a>` (the HTTP crate does
this with a `TryFrom<Decoded> for RoutedMessage` impl):

```rust
impl<'ctx> ProtocolDecoder<CoapResponse> for CoapResponseDecoder<'ctx> {
    type Error = CoapCommandConversionError;
    type Decoded<'a>
        = CoapDecodedResponse<'a, 'ctx>
    where
        CoapResponse: 'a;

    fn decode<'a>(&self, native: &'a CoapResponse) -> Result<Self::Decoded<'a>, Self::Error> {
        unimplemented!()
    }
}
```

### 5. Pick driver capabilities

Implement only what CoAP has, from `core/src/router.rs`:

- `RequestResponse` — GET/PUT/POST exchanges (`type Request = CoapRequest`,
  `type Response = CoapResponse`).
- `EventSource` — Observe notifications pushed into an `EventSink`.
- `Lifecycle` — socket/DTLS session start and stop.
- `PubSub` — skip it; CoAP is not a broker protocol. (Remember: if you do
  implement it, `subscribe` sinks take `RoutedEvent<'static>` — that is one
  of the sanctioned `.into_owned()` boundaries.)

That's the whole surface: one endpoint config, one attributes struct, one
typed meta view, three codec trait impls, and the capability traits your
transport supports. Existing MQTT/HTTP/Modbus devices can immediately
bridge to the new protocol through the shared IR.
